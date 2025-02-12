use cargo_regression::{test, Args, TestExitCode};

#[tokio::main]
async fn main() -> TestExitCode {
  let args = Args::parse_from(std::env::args_os());
  test(args).await
}
