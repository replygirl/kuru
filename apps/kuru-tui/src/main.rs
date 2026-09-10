#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--internal-dolt-supervisor"))
    {
        return kuru_memory::server::supervisor_entry().await;
    }
    kuru::cli::run().await
}
