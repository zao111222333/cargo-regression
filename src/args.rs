use clap::Parser;
use std::{
  collections::HashSet,
  ffi::OsString,
  mem::take,
  path::{Path, PathBuf},
};

use crate::regression::BuildError;

#[derive(Debug, Parser)]
#[command(version)]
pub struct Args {
  #[clap(long, help = "Debug mode flag, recommended")]
  pub(crate) debug: bool,
  #[clap(long, help = "Print errors [default: false, save errs to report]")]
  pub(crate) print_errs: bool,
  #[clap(long, help = "Default executable path", default_value_t = String::new())]
  pub(crate) exe_path: String,
  #[clap(long, help = "Default arguements", num_args = 1..)]
  pub(crate) args: Vec<String>,
  #[clap(long, help="Default input extensions(s)", num_args = 1..)]
  pub(crate) extensions: Vec<String>,
  #[clap(long, help="Input include. E.g., --include ./cases/*", num_args = 1..)]
  include: Vec<PathBuf>,
  #[clap(skip)]
  include_set: HashSet<PathBuf>,
  #[clap(long, help="Input exclude. E.g., --exclude ./cases/*", num_args = 1..)]
  exclude: Vec<PathBuf>,
  #[clap(skip)]
  exclude_set: HashSet<PathBuf>,
  #[clap(long, help = "Total permits to limit max parallelism", default_value_t = 1)]
  pub(crate) permits: u32,
  #[clap(long, help = "Change the directory to perform test", default_value = "./tmp")]
  pub(crate) work_dir: PathBuf,
  #[clap(value_parser)]
  pub(crate) root_dir: PathBuf,
  #[clap(skip)]
  pub(crate) root_dir_abs: PathBuf,
}

impl Args {
  pub const fn debug(mut self) -> Self {
    self.debug = true;
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
  pub fn exe_path(mut self, exe_path: impl AsRef<str>) -> Self {
    self.exe_path = exe_path.as_ref().into();
    self
  }
  pub fn args(mut self, iter: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
    self.args = iter.into_iter().map(|s| s.as_ref().into()).collect();
    self
  }
  pub fn work_dir(mut self, dir: impl AsRef<Path>) -> Self {
    self.work_dir = dir.as_ref().to_path_buf();
    self
  }
  pub fn extensions(mut self, iter: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
    self.extensions = iter.into_iter().map(|s| s.as_ref().into()).collect();
    self
  }
  pub fn include(mut self, iter: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
    self.include = iter.into_iter().map(|s| s.as_ref().to_path_buf()).collect();
    self
  }
  pub fn exclude(mut self, iter: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
    self.exclude = iter.into_iter().map(|s| s.as_ref().to_path_buf()).collect();
    self
  }
  pub fn new(root_dir: impl AsRef<str>) -> Self {
    <Self as Parser>::parse_from(["", root_dir.as_ref()])
  }
  pub fn parse_from<I, T>(itr: I) -> Self
  where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
  {
    <Self as Parser>::parse_from(itr)
  }
  pub(crate) fn rebuild(mut self) -> Result<&'static Self, BuildError> {
    self.root_dir_abs = std::fs::canonicalize(&self.root_dir)
      .map_err(|e| BuildError::ReadDir(self.root_dir.to_path_buf(), e))?;
    self.include_set = take(&mut self.include)
      .into_iter()
      .map(|path| match std::fs::canonicalize(&path) {
        Ok(p) => Ok(p),
        Err(e) => Err(BuildError::ReadDir(path, e)),
      })
      .collect::<Result<HashSet<_>, _>>()?;
    self.exclude_set = take(&mut self.exclude_set)
      .into_iter()
      .map(|path| match std::fs::canonicalize(&path) {
        Ok(p) => Ok(p),
        Err(e) => Err(BuildError::ReadDir(path, e)),
      })
      .collect::<Result<HashSet<_>, _>>()?;
    if self.extensions.iter().any(|s| s == "toml") {
      return Err(BuildError::InputExtToml);
    }
    Ok(Box::leak(Box::new(self)))
  }
  pub(super) fn filtered(&self, file: &Path) -> Result<bool, BuildError> {
    let file_abs = std::fs::canonicalize(file)
      .map_err(|e| BuildError::ReadDir(file.to_path_buf(), e))?;
    let included =
      if self.include_set.is_empty() { true } else { self.include_set.contains(&file_abs) };
    let excluded =
      if self.exclude_set.is_empty() { false } else { self.exclude_set.contains(&file_abs) };
    Ok(!included || excluded)
  }
}
