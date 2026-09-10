//! A compiled fixture with the real memory-owner lifetime and private handshake.
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use kuru_memory::server::{Server, ServerOptions};
    use std::{path::PathBuf, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments
        .get(1)
        .is_some_and(|argument| argument == "--internal-dolt-supervisor")
    {
        return kuru_memory::server::supervisor_entry().await;
    }
    anyhow::ensure!(
        arguments.len() == 4,
        "expected fixture root, engine and private rendezvous"
    );
    let root = PathBuf::from(&arguments[1]);
    let mut parent =
        kuru_platform::windows::pipe::connect(&arguments[3], Duration::from_secs(5)).await?;
    let server = Server::open(ServerOptions {
        binary: PathBuf::from(&arguments[2]),
        directory: root.join("store café 東京"),
        project_scope: "native-owner-fixture".into(),
        supervisor: std::env::current_exe()?,
        timeout: Duration::from_secs(25),
        read_only: false,
        retained: None,
        lifecycle_root: Some(root.join("lifecycles")),
    })
    .await?;
    let pool = server.pool("main").await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS owner_receipt (id INT PRIMARY KEY, value TEXT NOT NULL)",
    )
    .execute(pool.as_ref())
    .await?;
    sqlx::query("REPLACE INTO owner_receipt VALUES (1, 'accepted before creator loss')")
        .execute(pool.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'owner fixture accepted receipt', '--author', 'Kuru tests <tests@example.invalid>')").execute(pool.as_ref()).await?;
    parent.write_all(b"READY\n").await?;
    parent.flush().await?;
    let mut byte = [0];
    tokio::time::timeout(Duration::from_secs(45), parent.read(&mut byte))
        .await
        .context("fixture parent did not complete its ownership scenario")??;
    pool.close().await;
    server.close().await?;
    parent.close(Duration::from_secs(3)).await?;
    Ok(())
}
