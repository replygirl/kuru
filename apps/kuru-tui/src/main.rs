#[tokio::main]
async fn main() -> anyhow::Result<()> {
    kuru::cli::run().await
}
