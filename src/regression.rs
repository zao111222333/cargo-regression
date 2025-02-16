use core::fmt;
use std::{
  io,
  path::PathBuf,
  process::{ExitCode, Termination},
  sync::Arc,
  time::{Duration, Instant},
};

use itertools::{Either, Itertools};
use tokio::{fs::remove_dir_all, sync::Semaphore};

use crate::{
  assert::{AssertError, DisplayErrs},
  config::FullConfig,
  Args,
};

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
      Self::Ok(None) => write!(f, "\x1B[32mok\x1B[0m"),
      Self::Ok(Some(time)) => write!(f, "{:.2}s \x1B[32mok\x1B[0m", time.as_secs_f32()),
      Self::Failed(Some((_, time))) => {
        write!(f, "{:.2}s \x1B[31mFAILED\x1B[0m", time.as_secs_f32())
      }
      Self::Failed(None) => write!(f, "\x1B[31mFAILED\x1B[0m"),
      Self::Ignored => write!(f, "\x1B[33mignored\x1B[0m"),
      Self::FilteredOut => write!(f, "\x1B[2mfiltered out\x1B[0m"),
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
        if faileds.is_empty() {
          println!("test result: {}. {count_ok} passed; 0 failed; {count_ignored} ignored; {count_filtered} filtered out; finished in {time:.2}s", State::Ok(None));
          ExitCode::SUCCESS
        } else {
          print!("\nfailures:");
          for failed in &faileds {
            print!("{failed}");
          }
          println!("\n\ntest result: {}. {count_ok} passed; {} failed; {count_ignored} ignored; {count_filtered} filtered out; finished in {time:.2}s", State::Failed(None), faileds.len());
          ExitCode::FAILURE
        }
      }
      Err(build_errs) => {
        println!("Fail to build test:");
        for err in &build_errs {
          println!("{err}");
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
  let f1 = async move {
    if args.work_dir.exists() {
      remove_dir_all(&args.work_dir)
        .await
        .map_err(|e| BuildError::CleanDir(args.work_dir.to_path_buf(), e))
    } else {
      Ok(())
    }
  };
  let f2 =
    async move { walk(FullConfig::new(args), args.root_dir.to_path_buf(), args).await };
  // walkthrough all config
  let (clean_dir, file_configs) = tokio::join!(f1, f2);
  if let Err(e) = clean_dir {
    return Err(vec![e]);
  }
  let scheduler = Arc::new(Semaphore::new(args.permits as usize));
  let handles: Vec<_> = file_configs?
    .into_iter()
    .map(|(path, config)| {
      let scheduler = scheduler.clone();
      tokio::spawn(async move {
        let _permit = scheduler
          .acquire_many(*config.permit)
          .await
          .expect("Semaphore closed");
        let state = config.test(&path, args).await;
        println!("test {} ... {}", path.display(), state);
        state
      })
    })
    .collect();

  let mut count_ok = 0;
  let mut count_ignored = 0;
  let mut count_filtered = 0;
  let mut faileds = Vec::with_capacity(handles.len());
  // TODO: iter
  for handle in handles {
    match handle.await.unwrap() {
      State::Ok(Some(_)) => count_ok += 1,
      State::Failed(Some((failed, _))) => faileds.push(failed),
      State::Ok(None) | State::Failed(None) => unreachable!(),
      State::Ignored => count_ignored += 1,
      State::FilteredOut => count_filtered += 1,
    }
  }
  scheduler.close();
  Ok(TestResult { count_ok, count_ignored, count_filtered, faileds })
}

#[async_recursion::async_recursion]
async fn walk(
  mut current_config: FullConfig,
  current_path: PathBuf,
  args: &'static Args,
) -> Result<Vec<(PathBuf, FullConfig)>, Vec<BuildError>> {
  let all_path = current_path.join("__all__.toml");
  if all_path.exists() {
    match current_config.update(&all_path, args.debug) {
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
        if path.file_name().unwrap() == "__golden__" {
          Either::Left(None)
        } else {
          let current_config = current_config.clone();
          Either::Left(Some(tokio::spawn(async move {
            walk(current_config, path, args).await
          })))
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
                match current_config.update(&config_file, args.debug) {
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
  if errs.is_empty() {
    Ok(file_configs)
  } else {
    Err(errs)
  }
}
