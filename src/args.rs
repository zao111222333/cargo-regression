use clap::Parser;
use std::{
  ffi::{OsStr, OsString},
  path::{Path, PathBuf},
};

use crate::regression::BuildError;

#[derive(Debug, Clone, Copy)]
pub struct Args {
  pub(crate) debug: bool,
  pub(crate) regolden: bool,
  pub(crate) print_errs: bool,
  /// to schedule tasks, default is 1
  pub(crate) permits: u32,
  pub(crate) exe_path: &'static str,
  pub(crate) args: &'static [&'static str],
  pub(crate) work_dir: &'static Path,
  pub(crate) root_dir: &'static Path,
  // #[setters(skip)]
  pub(crate) root_dir_abs: &'static Path,
  // TODO: use static hashset
  pub(crate) extensions: &'static [&'static str],
  // TODO: use static hashset
  pub(crate) include: &'static [&'static Path],
  pub(crate) exclude: &'static [&'static Path],
}

#[derive(Debug, Parser)]
#[command(version)]
// , separated by space
struct ArgsBuilder {
  #[clap(long, help = "Debug mode flag, recommended")]
  debug: bool,
  #[clap(long, help = "Regolden mode flag")]
  regolden: bool,
  #[clap(long, help = "Print errors [default: false, save errs to report]")]
  print_errs: bool,
  #[clap(long, help = "Default executable path")]
  exe_path: Option<String>,
  #[clap(long, help = "Default arguements", num_args = 1..)]
  args: Vec<String>,
  #[clap(long, help="Default input extensions(s)", num_args = 1..)]
  extensions: Vec<String>,
  #[clap(long, help="Input include. E.g., --include ./cases/*", default_value = None, num_args = 1..)]
  include: Vec<String>,
  #[clap(long, help="Input exclude. E.g., --exclude ./cases/*", default_value = None, num_args = 1..)]
  exclude: Vec<String>,
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
  pub const fn debug(mut self) -> Self {
    self.debug = true;
    self
  }
  pub const fn regolden(mut self) -> Self {
    self.regolden = true;
    self
  }
  pub const fn print_errs(mut self) -> Self {
    self.print_errs = true;
    self
  }
  pub const fn permits(mut self, permits: u32) -> Self {
    self.permits = permits;
    self
  }
  pub const fn exe_path(mut self, exe_path: &'static str) -> Self {
    self.exe_path = exe_path;
    self
  }
  pub const fn args(mut self, args: &'static [&'static str]) -> Self {
    self.args = args;
    self
  }
  pub fn work_dir(mut self, dir: &'static str) -> Self {
    self.work_dir = Path::new(dir);
    self
  }
  pub fn root_dir(mut self, dir: &'static str) -> Self {
    self.root_dir = Path::new(dir);
    self
  }
  pub const fn extensions(mut self, extensions: &'static [&'static str]) -> Self {
    self.extensions = extensions;
    self
  }
  pub fn include(mut self, files: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
    self.include = leak_path_vec(files);
    self
  }
  pub fn exclude(mut self, files: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
    self.exclude = leak_path_vec(files);
    self
  }
  pub fn new() -> Self {
    Self {
      debug: false,
      print_errs: false,
      regolden: false,
      permits: 1,
      exe_path: "",
      args: &[],
      work_dir: Path::new(""),
      root_dir: Path::new(""),
      root_dir_abs: Path::new(""),
      extensions: &[],
      include: &[],
      exclude: &[],
    }
  }
  pub(crate) fn rebuild(mut self) -> Result<Self, BuildError> {
    self.root_dir_abs = leak_path(
      std::fs::canonicalize(self.root_dir)
        .map_err(|e| BuildError::ReadDir(self.root_dir.to_path_buf(), e))?,
    );
    self.include = leak_path_vec_res(self.include.iter().map(|path| {
      match std::fs::canonicalize(path) {
        Ok(p) => Ok(p),
        Err(e) => Err(BuildError::ReadDir(path.to_path_buf(), e)),
      }
    }))?;
    self.exclude = leak_path_vec_res(self.exclude.iter().map(|path| {
      match std::fs::canonicalize(path) {
        Ok(p) => Ok(p),
        Err(e) => Err(BuildError::ReadDir(path.to_path_buf(), e)),
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
    Args {
      debug: builder.debug,
      regolden: builder.regolden,
      print_errs: builder.print_errs,
      permits: builder.permits,
      exe_path: builder.exe_path.map_or("", leak_string),
      args: leak_string_vec(builder.args),
      extensions: leak_string_vec(builder.extensions),
      include: leak_path_vec(builder.include),
      exclude: leak_path_vec(builder.exclude),
      work_dir: leak_path(builder.work_dir),
      root_dir: leak_path(builder.root_dir),
      root_dir_abs: Path::new(""),
    }
  }
  pub(super) fn filtered(&self, file: &Path) -> Result<bool, BuildError> {
    let file_abs = std::fs::canonicalize(file)
      .map_err(|e| BuildError::ReadDir(file.to_path_buf(), e))?;
    let included = if self.include.is_empty() {
      true
    } else {
      self.include.iter().any(|pattern| *pattern == file_abs)
    };
    let excluded = if self.exclude.is_empty() {
      false
    } else {
      self.exclude.iter().any(|pattern| *pattern == file_abs)
    };
    Ok(!included || excluded)
  }
}

fn leak_string(s: String) -> &'static str {
  Box::leak(s.into_boxed_str())
}
fn leak_path(s: impl AsRef<Path>) -> &'static Path {
  Box::leak(s.as_ref().to_path_buf().into_boxed_path())
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
fn leak_path_vec(
  iter: impl IntoIterator<Item = impl AsRef<Path>>,
) -> &'static [&'static Path] {
  Box::leak(iter.into_iter().map(leak_path).collect::<Vec<_>>().into_boxed_slice())
}
fn leak_path_vec_res<E>(
  iter: impl IntoIterator<Item = Result<PathBuf, E>>,
) -> Result<&'static [&'static Path], E> {
  fn leak_path(s: PathBuf) -> &'static Path {
    Box::leak(s.into_boxed_path())
  }
  Ok(Box::leak(
    iter
      .into_iter()
      .map(|res| res.map(leak_path))
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
