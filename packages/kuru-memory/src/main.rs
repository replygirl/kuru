#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let argument = std::env::args().nth(1);
    match argument.as_deref() {
        Some("--internal-dolt-supervisor") => kuru_memory::server::supervisor_entry().await,
        Some("--internal-memory-service") => {
            kuru_memory::service::service_entry(std::env::args_os().skip(2)).await
        }
        #[cfg(feature = "test-support")]
        Some("--internal-memory-service-client-fixture") => {
            kuru_memory::service::client_fixture_entry(std::env::args_os().skip(2)).await
        }
        #[cfg(feature = "test-support")]
        Some("prefetch") => {
            // Cargo may republish its top-level binary alias after this task
            // completes. Ordinary test processes use this immutable snapshot.
            kuru_memory::test_support::prepare_supervisor()?;
            let cache = std::env::var_os("KURU_DOLT_CACHE")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join("kuru-dolt-test-cache"));
            let binary = kuru_memory::provision::provision(&Default::default(), &cache).await?;
            println!("{}", binary.display());
            Ok(())
        }
        #[cfg(feature = "test-support")]
        _ => anyhow::bail!("expected prefetch or an internal service entry"),
        #[cfg(not(feature = "test-support"))]
        _ => anyhow::bail!("expected an internal service entry"),
    }
}
