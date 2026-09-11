#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--internal-update-helper"))
    {
        return kuru_delivery::update::run_helper(std::env::args_os().skip(2).collect()).await;
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--internal-dolt-supervisor"))
    {
        return kuru_memory::server::supervisor_entry().await;
    }
    kuru::cli::run().await
}
