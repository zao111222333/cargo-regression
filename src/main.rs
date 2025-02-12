use cargo_regression::{test, Args, TestExitCode};

#[tokio::main]
async fn main() -> TestExitCode {
  // in sub-command mode, skip the first arg
  let args = Args::parse_from(std::env::args_os().into_iter().skip(1));
  test(args).await
}
