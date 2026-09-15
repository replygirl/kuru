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

#[tokio::test]
#[ignore = "requires an exact staged Windows release candidate"]
async fn native_mise_installs_and_activates_exact_staged_windows_archive() {
    let archive = std::env::var_os("KURU_STAGED_WINDOWS_ARCHIVE")
        .map(std::path::PathBuf::from)
        .expect("KURU_STAGED_WINDOWS_ARCHIVE is required");
    acceptance::run_staged(&archive).await.unwrap();
}
