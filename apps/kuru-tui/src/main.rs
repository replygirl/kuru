#[cfg(not(windows))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Box::pin(dispatch()).await
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    const DISPATCH_STACK_BYTES: usize = 8 * 1024 * 1024;

    let worker = std::thread::Builder::new()
        .name("kuru-dispatch".into())
        .stack_size(DISPATCH_STACK_BYTES)
        .spawn(windows_dispatch)
        .map_err(|error| anyhow::anyhow!("start Kuru Windows dispatch worker: {error}"))?;
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("Kuru Windows dispatch worker panicked"))?
}

#[cfg(windows)]
#[tokio::main]
async fn windows_dispatch() -> anyhow::Result<()> {
    Box::pin(dispatch()).await
}

async fn dispatch() -> anyhow::Result<()> {
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
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--internal-memory-service"))
    {
        return kuru_memory::service::service_entry(std::env::args_os().skip(2)).await;
    }
    kuru::cli::run_with_diagnostics().await
}
