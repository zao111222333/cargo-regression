use cargo_regression::{Args, TestExitCode};

#[tokio::main]
async fn main() -> TestExitCode {
  let args = Args::new().debug().work_dir("tmp").root_dir("demo");
  args.test().await
}
