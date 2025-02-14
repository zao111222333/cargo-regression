use cargo_regression::{Args, TestExitCode};

#[tokio::main]
async fn main() -> TestExitCode {
  let args = Args::parse_from(std::env::args_os());
  args.test().await
}
