mod args;
mod assert;
mod config;
mod regression;
use assert::Assert;

pub use args::Args;
pub use regression::TestExitCode;

#[tokio::test]
async fn demo() -> TestExitCode {
  let args = Args::new()
    .debug()
    .work_dir("tmp")
    .root_dir("demo")
    .include(&["demo/test-premit/test2.sh"]);
  args.test().await
}
