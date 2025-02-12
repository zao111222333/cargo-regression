mod args;
mod assert;
mod config;
mod regression;
use assert::Assert;

pub use args::Args;
pub use regression::{test, TestExitCode};

#[tokio::test]
async fn demo() -> TestExitCode {
  let args = Args::new().debug(true).work_dir("tmp").root_dir("cases");
  test(args).await
}
