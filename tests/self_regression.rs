use cargo_regression::{Args, TestExitCode};

#[tokio::test]
async fn self_regression() -> TestExitCode {
  let args = Args::new("tests/self_regression")
    .debug()
    .workdir("tmp")
    .extensions(["sh"])
    .exe_path("bash");
  args.test().await
}
