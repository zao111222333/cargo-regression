use cargo_regression::{Args, TestExitCode};

#[tokio::main]
async fn main() -> TestExitCode {
  // in sub-command mode, skip the first arg
  let args = Args::parse_from(std::env::args_os().skip(1));
  args.test().await
}
