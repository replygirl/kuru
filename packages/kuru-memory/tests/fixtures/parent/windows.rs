use anyhow::{Context, Result, ensure};
use kuru_memory::{
    server::{Server, ServerOptions},
    test_support::windows::{environment, forced_engine_cleanup, partial_readiness},
};
use kuru_platform::{
    fs::{Directory, NameRetention, Privacy},
    windows::{
        pipe::{self, PrivateListener},
        process::{NativeSpawnSpec, StandardStream, inherited_stdio},
    },
};
use std::{ffi::OsString, io::Write, path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn run() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().collect();
    match arguments.get(1).and_then(|value| value.to_str()) {
        Some("--internal-dolt-supervisor") => {
            return kuru_memory::server::supervisor_entry().await;
        }
        Some("--stubborn-root" | "--stubborn-leaf") => {
            return tokio::time::timeout(Duration::from_secs(60), stubborn(&arguments)).await?;
        }
        _ => {}
    }
    ensure!(
        arguments.len() == 5,
        "expected fixture root, engine, rendezvous and mode"
    );
    let root = PathBuf::from(&arguments[1]);
    let mode = arguments[4].to_str().context("fixture mode is not text")?;
    let mut parent = pipe::connect(&arguments[3], Duration::from_secs(5)).await?;
    if mode == "escalation" {
        forced_engine_cleanup(&root, &std::env::current_exe()?, &mut parent).await?;
        parent.close(Duration::from_secs(3)).await?;
        return Ok(());
    }
    let options = ServerOptions {
        binary: PathBuf::from(&arguments[2]),
        directory: root.join("store café 東京"),
        project_scope: "native-owner-fixture".into(),
        supervisor: std::env::current_exe()?,
        timeout: Duration::from_secs(25),
        read_only: false,
        retained: None,
        lifecycle_root: Some(root.join("lifecycles")),
    };
    if mode == "partial-ready" {
        partial_readiness(options, &mut parent).await?;
        parent.close(Duration::from_secs(3)).await?;
        return Ok(());
    }
    ensure!(
        matches!(mode, "committed" | "inflight"),
        "unknown fixture mode"
    );
    let server = Server::open(options).await?;
    let pool = server.pool("main").await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS owner_receipt (id INT PRIMARY KEY, value TEXT NOT NULL)",
    )
    .execute(pool.as_ref())
    .await?;
    sqlx::query("REPLACE INTO owner_receipt VALUES (1, 'accepted before creator loss')")
        .execute(pool.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'owner fixture accepted receipt', '--author', 'Kuru tests <tests@example.invalid>')")
        .execute(pool.as_ref()).await?;
    let pending = if mode == "inflight" {
        let mut transaction = pool.begin().await?;
        let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
            .fetch_one(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO owner_receipt VALUES (2, 'unfinished transaction')")
            .execute(&mut *transaction)
            .await?;
        let query = tokio::spawn(async move {
            // Dropping this task rolls back; there is deliberately no COMMIT.
            let _ = sqlx::query("SELECT SLEEP(30)")
                .execute(&mut *transaction)
                .await;
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let query: Option<String> = sqlx::query_scalar(
                    "SELECT INFO FROM information_schema.processlist WHERE ID = ?",
                )
                .bind(id)
                .fetch_optional(pool.as_ref())
                .await?;
                if query.is_some_and(|query| query.contains("SLEEP(30)")) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("accepted query never entered the actual process list")??;
        Some(query)
    } else {
        None
    };
    parent.write_all(b"READY\n").await?;
    parent.flush().await?;
    let mut byte = [0];
    tokio::time::timeout(Duration::from_secs(45), parent.read(&mut byte))
        .await
        .context("fixture parent did not complete its ownership scenario")??;
    if let Some(query) = pending {
        query.abort();
        let _ = query.await;
    }
    pool.close().await;
    server.close().await?;
    parent.close(Duration::from_secs(3)).await?;
    Ok(())
}

async fn stubborn(arguments: &[OsString]) -> Result<()> {
    let mut signal = tokio::signal::windows::ctrl_break()?;
    let root = PathBuf::from(&arguments[2]);
    if arguments[1] == "--stubborn-leaf" {
        let directory = Directory::open(&root, Privacy::OwnerOnly, NameRetention::Movable)?;
        let lock = directory.lock_file(std::ffi::OsStr::new("descendant.lock"))?;
        lock.lock()?;
        let mut parent = pipe::connect(&arguments[3], Duration::from_secs(5)).await?;
        parent.write_all(b"READY\n").await?;
        parent.flush().await?;
        signal.recv().await.context("console break stream ended")?;
        println!("BREAK-leaf");
        std::io::stdout().flush()?;
        // Deliberately retain both the data lock and the inherited output pipe.
        std::future::pending::<()>().await;
        drop((parent, lock, directory));
    } else {
        let listener = PrivateListener::bind()?;
        let mut command = NativeSpawnSpec::new(std::env::current_exe()?, root.clone());
        command.args = vec![
            "--stubborn-leaf".into(),
            root.into_os_string(),
            listener.address().into(),
        ];
        command.environment = environment()?;
        command.stdout = inherited_stdio(StandardStream::Output)?;
        let child = command.spawn().await?;
        let mut channel = listener.accept(&child, Duration::from_secs(5)).await?;
        let mut ready = [0; 6];
        tokio::time::timeout(Duration::from_secs(5), channel.read_exact(&mut ready)).await??;
        ensure!(
            &ready == b"READY\n",
            "stubborn descendant did not acquire its lock"
        );
        println!("READY");
        std::io::stdout().flush()?;
        signal.recv().await.context("console break stream ended")?;
        println!("BREAK-root");
        std::io::stdout().flush()?;
        std::future::pending::<()>().await;
        drop((channel, child));
    }
    Ok(())
}
