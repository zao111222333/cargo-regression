use core::fmt;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::{
  borrow::Cow,
  collections::HashSet,
  ffi::OsStr,
  fs::{create_dir_all, read_to_string, remove_dir_all},
  io::Write as _,
  iter::once,
  ops::{Deref, DerefMut},
  path::{Path, PathBuf},
  process::{Command, Output},
  sync::Arc,
  time::Instant,
};

use crate::{
  Args, Assert,
  assert::{AssertConfig, AssertError, DisplayErrs},
  regression::{BuildError, FailedState, GOLDEN_DIR, State},
};

#[derive(Default, Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(transparent)]
pub struct Source<T> {
  #[serde(skip)]
  source: Vec<String>,
  inner: T,
}
struct SourceDislay<'a>(&'a Vec<String>);
impl fmt::Display for SourceDislay<'_> {
  #[inline]
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for line in self.0 {
      writeln!(f, "# {line}")?;
    }
    Ok(())
  }
}
impl<T> Source<T> {
  fn source_display(&self) -> SourceDislay<'_> {
    SourceDislay(&self.source)
  }
  fn fmt_source<P: AsRef<Path>>(p: P) -> String {
    p.as_ref().to_path_buf().display().to_string()
  }
  fn add_source<P: AsRef<Path>>(&mut self, p: P, debug: bool) {
    if debug {
      self.source.push(Self::fmt_source(p));
    }
  }
}
impl<T, P: AsRef<Path>> From<(T, P, bool)> for Source<T> {
  #[inline]
  fn from(value: (T, P, bool)) -> Self {
    Self {
      source: if value.2 { vec![Self::fmt_source(value.1)] } else { vec![] },
      inner: value.0,
    }
  }
}
impl<T> From<T> for Source<T> {
  #[inline]
  fn from(inner: T) -> Self {
    Self { source: vec![], inner }
  }
}

impl<T> Deref for Source<T> {
  type Target = T;
  #[inline]
  fn deref(&self) -> &Self::Target {
    &self.inner
  }
}

impl<T> DerefMut for Source<T> {
  #[inline]
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.inner
  }
}
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct PrePostProcess {
  cmd: String,
  args: Option<Vec<String>>,
  workdir: Option<String>,
}
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct FullConfig {
  #[serde(skip)]
  name: String,
  #[serde(skip)]
  extension: String,
  #[serde(skip)]
  filtered: bool,
  #[serde(skip)]
  ignore: Source<bool>,
  pub(crate) preprocess: Source<Vec<PrePostProcess>>,
  pub(crate) postprocess: Source<Vec<PrePostProcess>>,
  print_errs: Source<bool>,
  pub(crate) permit: Source<u32>,
  cmd: Source<String>,
  args: Source<Vec<String>>,
  envs: Source<IndexMap<String, String>>,
  epsilon: Source<f32>,
  pub(crate) extensions: Source<HashSet<String>>,
  /// In default, only link all `{{name}}*` files into workdir.
  /// Use it to specify extern files.
  extern_files: Source<Vec<String>>,
  assert: Source<Assert>,
}

#[derive(Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct Config {
  ignore: Option<bool>,
  print_errs: Option<bool>,
  permit: Option<u32>,
  cmd: Option<String>,
  preprocess: Option<Vec<PrePostProcess>>,
  postprocess: Option<Vec<PrePostProcess>>,
  extensions: Option<HashSet<String>>,
  epsilon: Option<f32>,
  args: Option<Vec<String>>,
  envs: Option<IndexMap<String, String>>,
  extern_files: Option<Vec<String>>,
  extend: Option<Extend>,
  assert: Option<Assert>,
}

impl FullConfig {
  pub(crate) fn new_filtered() -> Self {
    Self { filtered: true, ..Default::default() }
  }
  pub(crate) fn new(args: &'static Args) -> Self {
    Self {
      cmd: args.cmd.clone().into(),
      print_errs: args.print_errs.into(),
      epsilon: 1e-10.into(),
      args: args.args.clone().into(),
      extensions: args.extensions.iter().cloned().collect::<HashSet<_>>().into(),
      ..Default::default()
    }
  }
  pub(crate) fn match_extension(&self, file: &Path) -> bool {
    file
      .extension()
      .and_then(OsStr::to_str)
      .and_then(|s| self.extensions.get(s))
      .is_some()
  }
  fn check(&self, file: &Path, args: &'static Args) -> Result<(), BuildError> {
    if *self.permit > args.permits {
      return Err(BuildError::PermitEcxceed(
        file.to_path_buf(),
        *self.permit,
        args.permits,
      ));
    }
    if self.cmd.is_empty() {
      return Err(BuildError::MissConfig(file.to_path_buf(), "cmd"));
    }
    if self.extensions.is_empty() {
      return Err(BuildError::MissConfig(file.to_path_buf(), "extensions"));
    }
    Ok(())
  }
  pub(crate) fn eval(
    mut self,
    file: &Path,
    args: &'static Args,
  ) -> Result<Self, BuildError> {
    self.check(file, args)?;
    self.extension = file.extension().unwrap().to_str().unwrap().to_owned();
    let name = file.with_extension("");
    self.name = name.file_name().unwrap().to_str().unwrap().to_owned();
    let eval_str = |s: &mut String| -> Result<(), BuildError> {
      *s = s.replace("{{extension}}", &self.extension);
      *s = s.replace("{{name}}", &self.name);
      *s = s.replace("{{rootdir}}", args.rootdir_abs.to_str().unwrap());
      Ok(())
    };
    eval_str(&mut self.cmd)?;
    for preprocess in self.preprocess.iter_mut() {
      eval_str(&mut preprocess.cmd)?;
      if let Some(args) = preprocess.args.as_mut() {
        for arg in args.iter_mut() {
          eval_str(arg)?;
        }
      }
      if let Some(workdir) = preprocess.workdir.as_mut() {
        eval_str(workdir)?;
      }
    }
    for postprocess in self.postprocess.iter_mut() {
      eval_str(&mut postprocess.cmd)?;
      if let Some(args) = postprocess.args.as_mut() {
        for arg in args.iter_mut() {
          eval_str(arg)?;
        }
      }
      if let Some(workdir) = postprocess.workdir.as_mut() {
        eval_str(workdir)?;
      }
    }
    for args in self.args.iter_mut() {
      eval_str(args)?;
    }
    for extern_file in self.extern_files.iter_mut() {
      eval_str(extern_file)?;
    }
    for v in self.envs.values_mut() {
      eval_str(v)?;
    }
    self.envs.entry("name".to_owned()).insert_entry(self.name.clone());
    self
      .envs
      .entry("extension".to_owned())
      .insert_entry(self.extension.clone());
    self
      .envs
      .entry("rootdir".to_owned())
      .insert_entry(args.rootdir_abs.display().to_string());
    if let Some(goldens) = self.assert.golden.as_deref_mut() {
      for golden in goldens.iter_mut() {
        eval_str(&mut golden.file)?;
      }
    }
    Ok(self)
  }
  #[inline]
  pub(crate) fn update(
    mut self,
    config_path: &Path,
    debug: bool,
  ) -> Result<Self, BuildError> {
    let toml_str = read_to_string(config_path)
      .map_err(|e| BuildError::UnableToRead(config_path.to_path_buf(), e))?;
    let config = toml::from_str::<Config>(&toml_str)
      .map_err(|e| BuildError::Toml(config_path.to_path_buf(), e))?;
    if let Some(preprocess) = config.preprocess {
      self.preprocess = (preprocess, config_path, debug).into();
    }
    if let Some(postprocess) = config.postprocess {
      self.postprocess = (postprocess, config_path, debug).into();
    }
    if let Some(ignore) = config.ignore {
      self.ignore = (ignore, config_path, debug).into();
    }
    if let Some(print_errs) = config.print_errs {
      self.print_errs = (print_errs, config_path, debug).into();
    }
    if let Some(epsilon) = config.epsilon {
      self.epsilon = (epsilon, config_path, debug).into();
    }
    if let Some(extensions) = config.extensions {
      self.extensions = (extensions, config_path, debug).into();
    }
    if let Some(permit) = config.permit {
      self.permit = (permit, config_path, debug).into();
    }
    if let Some(cmd) = config.cmd {
      self.cmd = (cmd, config_path, debug).into();
    }
    if let Some(args) = config.args {
      self.args = (args, config_path, debug).into();
    }
    if let Some(envs) = config.envs {
      self.envs = (envs, config_path, debug).into();
    }
    if let Some(extern_files) = config.extern_files {
      self.extern_files = (extern_files, config_path, debug).into();
    }
    if let Some(assert) = config.assert {
      self.assert = (assert, config_path, debug).into();
    }
    if let Some(extend) = config.extend {
      if let Some(args) = extend.args {
        self.args.extend(args);
        self.args.add_source(config_path, debug);
      }
      if let Some(envs) = extend.envs {
        self.envs.extend(envs);
        self.envs.add_source(config_path, debug);
      }
      if let Some(extern_files) = extend.extern_files {
        self.extern_files.extend(extern_files);
        self.extern_files.add_source(config_path, debug);
      }
    }
    Ok(self)
  }
}

impl FullConfig {
  #[inline]
  pub(crate) async fn test(self, path: &Path, args: &'static Args) -> State {
    if self.filtered {
      return State::FilteredOut;
    }
    if *self.ignore {
      return State::Ignored;
    }
    let print_errs = *self.print_errs;
    let rootdir = path.parent().unwrap();
    let path_str = path.to_str().unwrap();
    let workdir = args.workdir.join(
      // remove the root of rootdir
      {
        let rootdir = args.rootdir.to_str().unwrap();
        if path_str.starts_with(rootdir) {
          let end_with_slash = rootdir.ends_with(if cfg!(windows) { '\\' } else { '/' });
          &path_str[rootdir.len() + if end_with_slash { 0 } else { 1 }..]
        } else {
          path_str
        }
      },
    );
    let now = Instant::now();
    let name = self.name.clone();
    let mut errs = if let Err(e) = self.prepare_dir(rootdir, &workdir) {
      vec![e]
    } else {
      let toml_str = if args.nodebug { String::new() } else { self.to_toml() };
      let debug_config = workdir.join(format!("__debug__.{name}.toml"));
      let task_future = self.assert(rootdir, workdir.clone());
      let debug_future = async {
        if args.nodebug {
          Ok(())
        } else {
          tokio::fs::write(&debug_config, toml_str)
            .await
            .map_err(|e| AssertError::Write(debug_config.display().to_string(), e))
        }
      };
      let (e, mut errs) = tokio::join!(debug_future, task_future);
      if let Err(e) = e {
        errs.push(e);
      }
      errs
    };
    if errs.is_empty() {
      State::Ok(Some(now.elapsed()))
    } else {
      let failed_state = if print_errs {
        FailedState::NoReport(path.to_path_buf(), errs)
      } else {
        let err_report = workdir.join(format!("{name}.report"));
        match tokio::fs::write(&err_report, DisplayErrs(&errs).to_string()).await {
          Ok(_) => FailedState::ReportSaved(err_report),
          Err(e) => FailedState::NoReport(path.to_path_buf(), {
            errs.push(AssertError::Write(err_report.display().to_string(), e));
            errs
          }),
        }
      };
      State::Failed(Some((failed_state, now.elapsed())))
    }
  }
  #[inline]
  fn to_toml(&self) -> String {
    toml::to_string(&self)
      .map(|s| {
        // TODO toml with comment
        s.replacen("args = [", &format!("{}args = [", self.args.source_display()), 1)
          .replacen("cmd = ", &format!("{}cmd = ", self.cmd.source_display()), 1)
          .replacen(
            "extern_files = ",
            &format!("{}extern_files = ", self.extern_files.source_display()),
            1,
          )
          .replacen(
            "[[preprocess]]",
            &format!("{}[[preprocess]]", self.preprocess.source_display()),
            1,
          )
          .replacen(
            "[[postprocess]]",
            &format!("{}[[postprocess]]", self.postprocess.source_display()),
            1,
          )
          .replacen("[envs]", &format!("{}[envs]", self.envs.source_display()), 1)
          .replace("[assert]", &format!("{}[assert]", self.assert.source_display()))
          .replace("[[assert", &format!("{}[[assert", self.assert.source_display()))
      })
      .unwrap_or_default()
  }
  #[inline]
  fn exec_process(&self, workdir: &Path, is_preprocess: bool) -> Result<(), AssertError> {
    let (processes, log_file_name) = if is_preprocess {
      (&self.preprocess, "__debug__.preprocess.log")
    } else {
      (&self.postprocess, "__debug__.postprocess.log")
    };
    if processes.is_empty() {
      return Ok(());
    }
    /// Wrapper
    #[derive(Debug)]
    #[expect(non_camel_case_types)]
    struct process<'s> {
      cmd: &'s str,
      args: &'s [String],
      workdir: &'s Path,
    }
    let log_file = workdir.join(log_file_name);
    let out_file = std::fs::File::create(&log_file)
      .map_err(|e| AssertError::UnableToCreateDir(log_file.display().to_string(), e))?;
    let mut writer = std::io::BufWriter::new(out_file);
    // exec all prepares
    for process in processes.iter() {
      let wrapper = process {
        cmd: &process.cmd,
        args: process.args.as_ref().map_or(&[], Vec::as_slice),
        workdir: process.workdir.as_ref().map_or(workdir, |workdir| Path::new(workdir)),
      };
      match Command::new(wrapper.cmd)
        .current_dir(wrapper.workdir)
        .args(wrapper.args)
        .envs(&*self.envs)
        .output()
      {
        Err(e) => return Err(AssertError::ProcessExec(format!("{wrapper:?}"), e)),
        Ok(output) => {
          if output.status.success() {
            writeln!(&mut writer, "[INFO] {wrapper:?}").unwrap();
          } else {
            write!(&mut writer, "[ERROR] {wrapper:?}\nstdout:\n").unwrap();
            writer.write_all(&output.stdout).unwrap();
            write!(&mut writer, "\nstderr:\n").unwrap();
            writer.write_all(&output.stderr).unwrap();
            writeln!(&mut writer).unwrap();
          }
        }
      }
    }
    Ok(())
  }
  #[inline]
  fn prepare_dir(&self, rootdir: &Path, workdir: &Path) -> Result<(), AssertError> {
    let rootdir = if rootdir.is_absolute() {
      Cow::Borrowed(rootdir)
    } else {
      Cow::Owned(
        std::fs::canonicalize(rootdir)
          .map_err(|e| AssertError::UnableToReadDir(rootdir.display().to_string(), e))?,
      )
    };
    // create
    if workdir.exists() {
      remove_dir_all(workdir)
        .map_err(|e| AssertError::UnableToDeleteDir(workdir.display().to_string(), e))?;
    }
    create_dir_all(workdir)
      .map_err(|e| AssertError::UnableToCreateDir(workdir.display().to_string(), e))?;
    // golden
    let golden_dir = rootdir.join(GOLDEN_DIR);
    if golden_dir.exists() {
      let link = workdir.join(GOLDEN_DIR);
      std::os::unix::fs::symlink(&golden_dir, &link).map_err(|e| {
        AssertError::LinkFile(
          golden_dir.display().to_string(),
          link.display().to_string(),
          e,
        )
      })?;
    }
    // extern_file
    for extern_file in self.extern_files.iter() {
      let path = rootdir.join(extern_file);
      if path.exists() {
        let link = workdir.join(extern_file);
        std::os::unix::fs::symlink(&path, &link).map_err(|e| {
          AssertError::LinkFile(path.display().to_string(), link.display().to_string(), e)
        })?;
      }
    }
    for entry in rootdir
      .read_dir()
      .map_err(|e| AssertError::UnableToReadDir(rootdir.display().to_string(), e))?
      .flatten()
    {
      let full_name = entry.file_name();
      if full_name.to_str().unwrap_or("").starts_with(&self.name) {
        let original = entry.path();
        let link = workdir.join(full_name);
        std::os::unix::fs::symlink(&original, &link).map_err(|e| {
          AssertError::LinkFile(
            original.display().to_string(),
            link.display().to_string(),
            e,
          )
        })?;
      }
    }
    self.exec_process(workdir, true)?;
    Ok(())
  }
  #[inline]
  fn exe(&self, workdir: &Path) -> Result<Output, AssertError> {
    let output = Command::new(&*self.cmd)
      .current_dir(workdir)
      .args(&*self.args)
      .envs(&*self.envs)
      .output()
      .map_err(|e| {
        AssertError::Executes(
          once(self.cmd.inner.clone())
            .chain(self.args.iter().cloned())
            .collect(),
          e,
        )
      })?;
    self.exec_process(workdir, false)?;
    Ok(output)
  }
  #[inline]
  async fn assert(self, rootdir: &Path, workdir: PathBuf) -> Vec<AssertError> {
    match self.exe(&workdir) {
      Ok(output) => {
        let assert_config = self.assert_config();
        self
          .assert
          .inner
          .assert(
            assert_config,
            self.name,
            workdir,
            rootdir.join(GOLDEN_DIR),
            Arc::new(output),
          )
          .await
      }
      Err(e) => vec![e],
    }
  }
  fn assert_config(&self) -> AssertConfig {
    AssertConfig { epsilon: *self.epsilon }
  }
}

#[derive(Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct Extend {
  args: Option<Vec<String>>,
  envs: Option<IndexMap<String, String>>,
  extern_files: Option<Vec<String>>,
}

#[test]
fn test_parse() {
  let toml_str = r#"
cmd = "python"
args = ["{{name}}.py", "var1", "var2"]
envs = { k1 = "v1", k2 = "v2" }

[extend]
args = ["var3", "var4"]
envs = { k3 = "v3", k4 = "v4" }

[assert]
exit-code = 1

[[assert.golden]]
file = "{{name}}.stderr"
match = [
  { pattern = ".*err", count = 1 },
  { pattern = ".*ok", count = 2 },
]

[[assert.golden]]
file = "{{name}}.stdout"
match = [
  { pattern = ".*ok", count_at_least = 1 }
]

[[assert.golden]]
file = "{{name}}.text"
equal = true

[[assert.golden]]
file = "out.text"
equal = true
      "#;
  let res = toml::from_str::<Config>(toml_str);
  match res {
    Ok(config) => {
      println!("{config:#?}");
    }
    Err(e) => println!("{e}"),
  }
}
