use anyhow::Result;
use kuru_memory::{
    server::{Server, ServerOptions},
    test_support,
};
use std::{fs, time::Duration};

#[tokio::test]
async fn prepared_snapshot_runs_real_supervisor_after_cargo_alias_disappears_and_changes()
-> Result<()> {
    let root = test_support::tempdir()?;
    let source = root.path().join(if cfg!(windows) {
        "cargo-alias.exe"
    } else {
        "cargo-alias"
    });
    fs::copy(env!("CARGO_BIN_EXE_kuru-memory"), &source)?;
    let supervisor = test_support::snapshot_supervisor(&source, root.path())?;
    fs::remove_file(&source)?;
    let binary = test_support::warm_runtime_cache().await?;
    let options = ServerOptions {
        binary,
        directory: root.path().join("actual store"),
        project_scope: "snapshot-lifetime-fixture".into(),
        supervisor,
        timeout: Duration::from_secs(30),
        read_only: false,
        retained: None,
        lifecycle_root: cfg!(windows).then(|| root.path().join("lifecycles")),
    };
    let server = Server::open(options.clone()).await?;
    let pool = server.pool("main").await?;
    sqlx::query("CREATE TABLE retained_snapshot (id INT PRIMARY KEY, value TEXT NOT NULL)")
        .execute(pool.as_ref())
        .await?;
    sqlx::query("INSERT INTO retained_snapshot VALUES (1, 'survives artifact republication')")
        .execute(pool.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'snapshot fixture', '--author', 'Kuru tests <tests@example.invalid>')")
        .execute(pool.as_ref()).await?;
    pool.close().await;
    server.close().await?;
    fs::write(&source, b"unrelated replacement is never executed")?;
    let reopened = Server::open(options).await?;
    let pool = reopened.pool("main").await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM retained_snapshot WHERE id=1")
            .fetch_one(pool.as_ref())
            .await?,
        "survives artifact republication"
    );
    pool.close().await;
    reopened.close().await?;
    assert_eq!(
        fs::read(source)?,
        b"unrelated replacement is never executed"
    );
    Ok(())
}
