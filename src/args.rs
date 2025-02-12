use clap::Parser;
use derive_setters::Setters;
use std::{
  ffi::{OsStr, OsString},
  path::{Path, PathBuf},
};

use crate::regression::BuildError;

#[derive(Debug, Clone, Copy)]
#[derive(Setters)]
pub struct Args {
  pub(crate) debug: bool,
  pub(crate) regolden: bool,
  /// to schedule tasks, default is 1
  pub(crate) permits: u32,
  pub(crate) exe_path: &'static str,
  pub(crate) args: &'static [&'static str],
  pub(crate) work_dir: &'static str,
  pub(crate) root_dir: &'static str,
  #[setters(skip)]
  pub(crate) root_dir_abs: &'static str,
  // TODO: use static hashset
  pub(crate) extensions: &'static [&'static str],
  // TODO: use static hashset
  pub(crate) filter: &'static [&'static str],
}

#[derive(Debug, Parser)]
#[command(version)]
// , separated by space
struct ArgsBuilder {
  #[clap(long, help = "Debug mode flag, recommended")]
  debug: bool,
  #[clap(long, help = "Regolden mode flag")]
  regolden: bool,
  #[clap(long, help = "Default executable path")]
  exe_path: Option<String>,
  #[clap(long, help = "Default arguements", num_args = 1..)]
  args: Vec<String>,
  #[clap(long, help="Default input extensions(s)", num_args = 1..)]
  extensions: Vec<String>,
  #[clap(long, help="Input filter. E.g., --filter ./cases/*", default_value = None, num_args = 1..)]
  filter: Vec<String>,
  #[clap(long, help = "Total permits to limit max parallelism", default_value_t = 1)]
  permits: u32,
  #[clap(long, default_value_t = String::from("./tmp"))]
  work_dir: String,
  #[clap(value_parser)]
  root_dir: String,
}

impl Default for Args {
    fn default() -> Self {
        Self::new()
    }
}

impl Args {
  pub const fn new() -> Self {
    Self {
      debug: false,
      regolden: false,
      permits: 1,
      exe_path: "",
      args: &[],
      work_dir: "",
      root_dir: "",
      root_dir_abs: "",
      extensions: &[],
      filter: &[],
    }
  }
  pub(crate) fn rebuild(mut self) -> Result<Self, BuildError> {
    self.root_dir_abs = leak_string(
      std::fs::canonicalize(self.root_dir)
        .map_err(|e| BuildError::ReadDir(PathBuf::from(&self.root_dir), e))?
        .display()
        .to_string(),
    );
    self.filter = leak_string_vec_res(self.filter.iter().map(|path| {
      let path = PathBuf::from(path);
      match std::fs::canonicalize(&path) {
        Ok(p) => Ok(p.display().to_string()),
        Err(e) => Err(BuildError::ReadDir(PathBuf::from(&path), e)),
      }
    }))?;
    if self.extensions.iter().any(|&s| s == "toml") {
      return Err(BuildError::InputExtToml);
    }
    Ok(self)
  }
  pub fn parse_from<I, T>(itr: I) -> Self
  where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
  {
    let builder = ArgsBuilder::parse_from(itr);
    let mut args = Args::new();
    if let Some(exe_path) = builder.exe_path {
      args = args.exe_path(leak_string(exe_path));
    }
    args
      .permits(builder.permits)
      .regolden(builder.regolden)
      .debug(builder.debug)
      .args(leak_string_vec(builder.args))
      .extensions(leak_string_vec(builder.extensions))
      .filter(leak_string_vec(builder.filter))
      .work_dir(leak_string(builder.work_dir))
      .root_dir(leak_string(builder.root_dir))
  }
  pub(super) fn filtered(&self, file: &Path) -> Result<bool, BuildError> {
    if self.filter.is_empty() {
      Ok(false)
    } else {
      let file_abs = std::fs::canonicalize(file)
        .map_err(|e| BuildError::ReadDir(PathBuf::from(&file), e))?
        .display()
        .to_string();
      Ok(!self.filter.iter().any(|pattern| *pattern == file_abs))
    }
  }
}

fn leak_string(s: String) -> &'static str {
  Box::leak(s.into_boxed_str())
}
fn leak_string_vec(iter: impl IntoIterator<Item = String>) -> &'static [&'static str] {
  Box::leak(
    iter
      .into_iter()
      .map(leak_string)
      .collect::<Vec<_>>()
      .into_boxed_slice(),
  )
}
fn leak_string_vec_res<E>(
  iter: impl IntoIterator<Item = Result<String, E>>,
) -> Result<&'static [&'static str], E> {
  Ok(Box::leak(
    iter
      .into_iter()
      .map(|res| res.map(leak_string))
      .collect::<Result<Vec<_>, _>>()?
      .into_boxed_slice(),
  ))
}

pub(crate) fn match_extension<I: Iterator<Item = impl AsRef<str>>>(
  file: &Path,
  extensions: I,
) -> bool {
  file
    .extension()
    .and_then(OsStr::to_str)
    .and_then(|s| {
      if extensions.into_iter().any(|ext_s| ext_s.as_ref() == s) {
        Some(())
      } else {
        None
      }
    })
    .is_some()
}
