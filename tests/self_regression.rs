use cargo_regression::{Args, TestExitCode};

#[tokio::test]
async fn self_regression() -> TestExitCode {
  let args = Args::new("tests/self_regression")
    .workdir("tmp")
    .extensions(["sh"])
    .cmd("bash");
  args.test().await
}
