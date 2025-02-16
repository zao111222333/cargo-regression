use cargo_regression::{Args, TestExitCode};

#[tokio::main]
async fn main() -> TestExitCode {
  let args = Args::new("demo").debug().workdir("tmp");
  args.test().await
}
