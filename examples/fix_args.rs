use cargo_regression::{test, Args, TestExitCode};

#[tokio::main]
async fn main() -> TestExitCode {
  let args = Args::new().debug(true).work_dir("tmp").root_dir("demo");
  test(args).await
}
