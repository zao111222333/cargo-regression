mod args;
mod assert;
mod config;
mod regression;
use assert::Assert;

pub use args::Args;
pub use regression::TestExitCode;

#[tokio::test]
async fn demo() -> TestExitCode {
  let args = Args::new("demo")
    .debug()
    .workdir("tmp")
    .include(["demo/test-premit/test2.sh"]);
  args.test().await
}
