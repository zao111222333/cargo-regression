use core::fmt;
use std::{
  io,
  path::PathBuf,
  process::{ExitCode, Termination},
  sync::Arc,
  time::{Duration, Instant},
};

use colored::Colorize;
use itertools::{Either, Itertools};
use tokio::{
  fs::remove_dir_all,
  sync::{Mutex, Semaphore},
};

use crate::{
  Args,
  assert::{AssertError, DisplayErrs},
  config::FullConfig,
};

pub(crate) const GOLDEN_DIR: &str = "__golden__";

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
  #[error("file \"{0}\": {1}")]
  Toml(PathBuf, toml::de::Error),
  #[error("task \"{0}\": its permit = {1}, exceed total permits = {2}")]
  PermitEcxceed(PathBuf, u32, u32),
  #[error("task \"{0}\": need to specify '{1}'")]
  MissConfig(PathBuf, &'static str),
  #[error("file \"{0}\": {1}")]
  UnableToRead(PathBuf, io::Error),
  #[error("read dir \"{0}\": {1}")]
  ReadDir(PathBuf, io::Error),
  #[error("clean dir \"{0}\": {1}")]
  CleanDir(PathBuf, io::Error),
  #[error("input extensions can not contains 'toml'")]
  InputExtToml,
}

#[derive(Debug)]
pub(crate) enum FailedState {
  ReportSaved(PathBuf),
  NoReport(PathBuf, Vec<AssertError>),
}
pub(crate) enum State {
  Ok(Option<Duration>),
  Failed(Option<(FailedState, Duration)>),
  Ignored,
  FilteredOut,
}

impl fmt::Display for FailedState {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::ReportSaved(report) => {
        write!(f, "\n     report: {}", report.display())
      }
      Self::NoReport(input, errs) => {
        write!(f, "\n----------- {} -----------\n{}", input.display(), DisplayErrs(errs))
      }
    }
  }
}
impl fmt::Display for State {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Ok(None) => write!(f, "{}", "ok".green()),
      Self::Ok(Some(time)) => write!(f, "{:.2}s {}", time.as_secs_f32(), "ok".green()),
      Self::Failed(Some((_, time))) => {
        write!(f, "{:.2}s {}", time.as_secs_f32(), "FAILED".red())
      }
      Self::Failed(None) => write!(f, "{}", "FAILED".red()),
      Self::Ignored => write!(f, "{}", "ignored".yellow()),
      Self::FilteredOut => write!(f, "{}", "filtered out".bright_black()),
    }
  }
}

pub(crate) struct TestResult {
  count_ok: usize,
  count_ignored: usize,
  count_filtered: usize,
  faileds: Vec<FailedState>,
}

pub struct TestExitCode(Result<TestResult, Vec<BuildError>>, Instant);

impl Termination for TestExitCode {
  fn report(self) -> ExitCode {
    let time = self.1.elapsed().as_secs_f32();
    match self.0 {
      Ok(TestResult { count_ok, count_ignored, count_filtered, faileds }) => {
        println!();
        let failed_num = faileds.len();
        if failed_num == 0 {
          println!(
            "test result: {}. {count_ok} passed; {failed_num} failed; {count_ignored} ignored; {count_filtered} filtered out; finished in {time:.2}s",
            State::Ok(None)
          );
          ExitCode::SUCCESS
        } else {
          eprint!("failures:");
          for failed in &faileds {
            eprint!("{failed}");
          }
          eprintln!(
            "\n\ntest result: {}. {count_ok} passed; {failed_num} failed; {count_ignored} ignored; {count_filtered} filtered out; finished in {time:.2}s",
            State::Failed(None)
          );
          ExitCode::FAILURE
        }
      }
      Err(build_errs) => {
        eprintln!("Fail to build test:");
        for err in &build_errs {
          eprintln!("{err}");
        }
        ExitCode::FAILURE
      }
    }
  }
}

impl Args {
  pub async fn test(self) -> TestExitCode {
    let now = Instant::now();
    TestExitCode(
      match self.rebuild() {
        Ok(args) => _test(args).await,
        Err(e) => Err(vec![e]),
      },
      now,
    )
  }
}
async fn _test(args: &'static Args) -> Result<TestResult, Vec<BuildError>> {
  let f1 = async {
    if args.workdir.exists() {
      remove_dir_all(&args.workdir)
        .await
        .map_err(|e| BuildError::CleanDir(args.workdir.to_path_buf(), e))
    } else {
      Ok(())
    }
  };
  let f2 = walk(FullConfig::new(args), args.rootdir.to_path_buf(), args);
  // walkthrough all config
  let (clean_dir, file_configs) = tokio::join!(f1, f2);
  if let Err(e) = clean_dir {
    return Err(vec![e]);
  }
  let file_configs = file_configs?;
  let faileds = Arc::new(Mutex::new(Vec::with_capacity(file_configs.len())));
  let scheduler = Arc::new(Semaphore::new(args.permits as usize));
  let handles: Vec<_> = file_configs
    .into_iter()
    .map(|(path, config)| {
      let scheduler = scheduler.clone();
      let faileds = faileds.clone();
      tokio::spawn(async move {
        let _permit = scheduler
          .acquire_many(*config.permit)
          .await
          .expect("Semaphore closed");
        let state = config.test(&path, args).await;
        println!("test {} ... {}", path.display(), state);
        match state {
          State::Ok(Some(_)) => (1, 0, 0),
          State::Failed(Some((failed, _))) => {
            faileds.lock().await.push(failed);
            (0, 0, 0)
          }
          State::Ok(None) | State::Failed(None) => unreachable!(),
          State::Ignored => (0, 1, 0),
          State::FilteredOut => (0, 0, 1),
        }
      })
    })
    .collect();
  let mut count_ok = 0;
  let mut count_ignored = 0;
  let mut count_filtered = 0;
  for handle in handles {
    let (ok, ignored, filtered) = handle.await.unwrap();
    count_ok += ok;
    count_ignored += ignored;
    count_filtered += filtered;
  }
  scheduler.close();
  Ok(TestResult {
    count_ok,
    count_ignored,
    count_filtered,
    faileds: Arc::try_unwrap(faileds).unwrap().into_inner(),
  })
}

#[async_recursion::async_recursion]
async fn walk(
  mut current_config: FullConfig,
  current_path: PathBuf,
  args: &'static Args,
) -> Result<Vec<(PathBuf, FullConfig)>, Vec<BuildError>> {
  let all_path = current_path.join("__all__.toml");
  if all_path.exists() {
    match current_config.update(&all_path, !args.nodebug) {
      Ok(_config) => current_config = _config,
      Err(e) => return Err(vec![e]),
    }
  }
  let read_dir = match current_path.read_dir() {
    Ok(read_dir) => read_dir,
    Err(e) => return Err(vec![BuildError::ReadDir(current_path, e)]),
  };
  let (sub_dir_futures, files): (Vec<_>, Vec<_>) =
    read_dir.into_iter().partition_map(|entry| {
      let path = entry.unwrap().path();
      if path.is_dir() {
        if path.file_name().unwrap() == GOLDEN_DIR {
          Either::Left(None)
        } else {
          let current_config = current_config.clone();
          Either::Left(Some(tokio::spawn(walk(current_config, path, args))))
        }
      } else {
        Either::Right(path)
      }
    });
  let mut errs = Vec::new();
  let mut file_configs = files
    .into_iter()
    .filter_map(|file| {
      if current_config.match_extension(&file) {
        match args.filtered(&file) {
          Ok(filtered) => {
            if filtered {
              Some((file, FullConfig::new_filtered()))
            } else {
              let config_file = file.with_extension("toml");
              let current_config = current_config.clone();
              if config_file.is_file() {
                match current_config.update(&config_file, !args.nodebug) {
                  Ok(config) => Some((file, config)),
                  Err(e) => {
                    errs.push(e);
                    None
                  }
                }
              } else {
                Some((file, current_config))
              }
              .and_then(|(file, config)| match config.eval(&file, args) {
                Ok(config) => Some((file, config)),
                Err(e) => {
                  errs.push(e);
                  None
                }
              })
            }
          }
          Err(e) => {
            errs.push(e);
            None
          }
        }
      } else {
        None
      }
    })
    .collect::<Vec<_>>();
  for f in sub_dir_futures.into_iter().flatten() {
    match f.await.expect("join handle") {
      Ok(res) => file_configs.extend(res),
      Err(e) => errs.extend(e),
    }
  }
  if errs.is_empty() { Ok(file_configs) } else { Err(errs) }
}
