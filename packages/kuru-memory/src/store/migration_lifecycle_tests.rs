use super::*;
use rusqlite::Connection as SqliteConnection;
use serde_json::json;

async fn assert_current_store(store: &MemoryStore) -> Result<()> {
    migrations::validate_active(&store.shared.server, &store.pool).await?;
    ensure!(
        migrations::version(&store.pool).await? == migrations::CURRENT_VERSION,
        "fixture did not reach the current schema"
    );
    let receipts: i64 = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT COUNT(*) FROM kuru_migrations").fetch_one(store.pool.as_ref()),
    )
    .await
    .context("fixture receipt query deadline exceeded")??;
    ensure!(
        receipts == 1,
        "fixture expected one exact migration receipt"
    );
    Ok(())
}

fn assert_no_staging_directory(options: &OpenOptions) -> Result<()> {
    let active = project_directory(&options.data_dir, &options.project_scope)?;
    let parent = active
        .parent()
        .context("fixture active path has no parent")?;
    let prefix = format!(
        "{}.staging-",
        active
            .file_name()
            .context("fixture active path has no name")?
            .to_string_lossy()
    );
    for entry in fs::read_dir(parent)? {
        ensure!(
            !entry?.file_name().to_string_lossy().starts_with(&prefix),
            "activated store retained a live staging directory"
        );
    }
    Ok(())
}

#[tokio::test]
async fn inspection_owned_old_schema_blocks_writer_without_mutation() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let scope = format!("project/{}", "7".repeat(64));
    let mut options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
    super::tests::released_v1(&options).await?;
    let directory = project_directory(&options.data_dir, &scope)?;
    let inspector = Server::open(ServerOptions {
        binary: provision::provision(&options.config, &options.data_dir.join("tools/dolt")).await?,
        directory,
        project_scope: scope,
        supervisor: options
            .supervisor
            .clone()
            .context("inspection fixture needs supervisor")?,
        timeout: Duration::from_secs(options.config.startup_timeout_secs),
        read_only: true,
        retained: None,
        lifecycle_root: cfg!(windows).then(|| options.data_dir.join("memory/lifecycles")),
    })
    .await?;
    let inspected = inspector.pool("main").await?;
    let head = revision(&inspected).await?;
    let refs: Vec<(String, String)> =
        sqlx::query_as("SELECT name, hash FROM dolt_branches ORDER BY BINARY name LIMIT 65")
            .fetch_all(inspected.as_ref())
            .await?;
    let status: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT table_name, staged, status FROM dolt_status ORDER BY BINARY table_name, staged, BINARY status LIMIT 65",
    )
    .fetch_all(inspected.as_ref())
    .await?;
    assert_eq!(migrations::version(&inspected).await?, 1);

    options.config.startup_timeout_secs = 1;
    options.config.validate()?;
    let error = tokio::time::timeout(Duration::from_secs(6), MemoryStore::open(options.clone()))
        .await
        .context("contending writer did not reach its bounded ownership result")?
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("memory lifecycle lock is held; refusing takeover"),
        "unexpected writer contention error: {error:#}"
    );
    assert_eq!(revision(&inspected).await?, head);
    assert_eq!(
        sqlx::query_as::<_, (String, String)>(
            "SELECT name, hash FROM dolt_branches ORDER BY BINARY name LIMIT 65",
        )
        .fetch_all(inspected.as_ref())
        .await?,
        refs
    );
    assert_eq!(
        sqlx::query_as::<_, (String, i64, String)>(
            "SELECT table_name, staged, status FROM dolt_status ORDER BY BINARY table_name, staged, BINARY status LIMIT 65",
        )
        .fetch_all(inspected.as_ref())
        .await?,
        status
    );
    assert_eq!(migrations::version(&inspected).await?, 1);
    inspected.close().await;
    inspector.close().await?;

    options.config.startup_timeout_secs = 30;
    let upgraded = MemoryStore::open(options).await?;
    assert_current_store(&upgraded).await?;
    upgraded.close().await?;
    Ok(())
}

#[tokio::test]
async fn fresh_and_byte_sensitive_wal_import_publish_current_receipts_once() -> Result<()> {
    let fresh_root = crate::test_support::tempdir()?;
    let fresh_scope = format!("project/{}", "8".repeat(64));
    let fresh_options =
        crate::test_support::open_options(fresh_root.path().to_owned(), fresh_scope.clone())?;
    let fresh = MemoryStore::open(fresh_options.clone()).await?;
    assert_current_store(&fresh).await?;
    let fresh_activation = read_activation(
        &project_directory(&fresh_options.data_dir, &fresh_scope)?,
        &fresh_scope,
    )?;
    assert_eq!(fresh_activation.initial_revision, fresh.revision().await?);
    assert_eq!(fresh_activation.migration, None);
    assert_no_staging_directory(&fresh_options)?;
    fresh.close().await?;

    let import_root = crate::test_support::tempdir()?;
    let scope = format!("project/{}", "9".repeat(64));
    let other_scope = format!("project/{}", "a".repeat(64));
    let source_path = import_root.path().join("memory.sqlite3");
    let source = SqliteConnection::open(&source_path)?;
    source.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA wal_autocheckpoint=0;
         PRAGMA application_id=1263882837;
         PRAGMA user_version=1;
         CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL);
         CREATE TABLE state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
    )?;
    source.pragma_update(None, "application_id", 0x4b55_5255_i64)?;
    let namespace = format!("{scope}/identity/部品");
    for (sequence, content) in [(3_i64, "first\0東京"), (9, "second café")] {
        source.execute(
            "INSERT INTO messages VALUES (?1,?2,'tool',?3)",
            rusqlite::params![sequence, namespace, content],
        )?;
    }
    source.execute(
        "INSERT INTO messages VALUES (10,?1,'user','other project')",
        [format!("{other_scope}/transcript")],
    )?;
    let key = format!("{scope}/preferences");
    let raw_json = " { \"mode\" : \"jungian\", \"nested\" : [1,true,null] } ";
    source.execute(
        "INSERT INTO state VALUES (?1,?2)",
        rusqlite::params![key, raw_json],
    )?;
    source.execute(
        "INSERT INTO state VALUES (?1,'\"kept\"')",
        [format!("{other_scope}/preferences")],
    )?;
    let wal_path = import_root.path().join("memory.sqlite3-wal");
    ensure!(
        wal_path.is_file(),
        "fixture did not retain its committed WAL"
    );
    let source_bytes = fs::read(&source_path)?;
    let wal_bytes = fs::read(&wal_path)?;
    let prepared = migration::prepare(import_root.path(), &scope)?
        .context("fixture legacy database was not detected")?;
    let snapshot_bytes = fs::read(&prepared.receipt.snapshot)?;

    let options = crate::test_support::open_options(import_root.path().to_owned(), scope.clone())?;
    let imported = MemoryStore::open(options.clone()).await?;
    assert_current_store(&imported).await?;
    assert_eq!(
        imported
            .history(&namespace, 10)
            .await?
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>(),
        ["first\0東京", "second café"]
    );
    assert_eq!(
        imported.get(&key).await?,
        Some(json!({
            "mode": "jungian",
            "nested": [1, true, null]
        }))
    );
    let raw_imported: String = sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
        .bind(key.as_bytes())
        .fetch_one(imported.pool.as_ref())
        .await?;
    assert_eq!(raw_imported, raw_json);
    assert!(
        imported
            .history(&format!("{other_scope}/transcript"), 10)
            .await?
            .is_empty()
    );
    let activation = read_activation(&project_directory(&options.data_dir, &scope)?, &scope)?;
    assert_eq!(activation.initial_revision, imported.revision().await?);
    assert_eq!(activation.migration, Some(prepared.receipt.clone()));
    assert_no_staging_directory(&options)?;
    imported.close().await?;

    assert_eq!(fs::read(&source_path)?, source_bytes);
    assert_eq!(fs::read(&wal_path)?, wal_bytes);
    assert_eq!(fs::read(&prepared.receipt.snapshot)?, snapshot_bytes);
    let reopened = MemoryStore::open(options).await?;
    assert_current_store(&reopened).await?;
    assert_eq!(reopened.revision().await?, activation.initial_revision);
    assert_eq!(
        reopened
            .revisions(20)
            .await?
            .iter()
            .filter(|revision| revision.message.starts_with("Upgrade Kuru memory schema 2"))
            .count(),
        1
    );
    reopened.close().await?;
    drop(source);
    Ok(())
}
