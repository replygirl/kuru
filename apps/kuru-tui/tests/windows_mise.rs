#![cfg(windows)]

#[path = "../../../packages/kuru-delivery/tests/support/mise_acceptance.rs"]
mod acceptance;

#[tokio::test]
async fn native_mise_github_backend_installs_and_activates_real_offline_kuru() {
    let binary = std::env::var_os("KURU_EMBEDDED_TEST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_BIN_EXE_kuru")));
    acceptance::run(&binary).await.unwrap();
}
