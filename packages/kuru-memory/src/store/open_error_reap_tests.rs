//! `MemoryStore::open` returning `Err` after it started Dolt means that Dolt
//! has been reaped: a caller may release or remove the directory at once.
//!
//! Each test forces one class of post-start open error and then makes one
//! non-waiting attempt on the store directory's lifecycle lease. The owned
//! supervisor holds that lease until it has reaped Dolt and exited, so the
//! attempt observes the supervisor itself rather than elapsed time.

use super::*;

fn lifecycle_root(options: &OpenOptions) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        let _ = options;
        None
    }
    #[cfg(windows)]
    {
        Some(options.data_dir.join("memory/lifecycles"))
    }
}

/// `Server::quiescence_at` rejects a zero wait; one millisecond is a single
/// attempt, far shorter than any Dolt stop. A violation panics here, before
/// the fixture root drops, so the report names this contract.
///
/// The lease is a real lifecycle flock taken in this process. Since round 3
/// the fixture root's teardown reads process-local quiescence records rather
/// than probing this lock; the attempt and the release both hold the lock
/// gate only to serialize against each other; see `crate::spawn_gate`.
async fn assert_reaped(options: &OpenOptions, directory: &Path) -> Result<()> {
    let root = lifecycle_root(options);
    let _gate = crate::spawn_gate::locking_async().await;
    match Server::quiescence_at(directory, root.as_deref(), Duration::from_millis(1)).await {
        Ok(lease) => {
            drop(lease);
            Ok(())
        }
        Err(error) => panic!(
            "MemoryStore::open returned an error while its Dolt server was still live: {error:#}"
        ),
    }
}

fn options(root: &crate::test_support::TempDir, digit: char) -> Result<OpenOptions> {
    crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", digit.to_string().repeat(64)),
    )
}

/// Staged open: an error on a staged server's main pool (the migrated stage's
/// open-sequence pool, past the whole startup budget).
#[tokio::test]
async fn staged_open_error_returns_only_after_its_server_is_reaped() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = options(&root, '4')?;
    let entered = Arc::new(AtomicBool::new(false));
    options.migrated_stage_pool_delay =
        Some((crate::test_support::server_start_budget(), entered.clone()));
    let error = crate::test_support::spawn_gated_open(options.clone())
        .await
        .expect_err("an expired staged main pool cannot open memory");
    assert!(entered.load(Ordering::SeqCst));
    assert!(
        format!("{error:#}").contains("open migrated staged main pool"),
        "the staged open failed before its migrated main pool"
    );
    let directory = project_directory(&options.data_dir, &options.project_scope)?;
    let prefix = format!(
        "{}.staging-",
        directory
            .file_name()
            .context("project store has no name")?
            .to_string_lossy()
    );
    let mut stages = Vec::new();
    for entry in fs::read_dir(directory.parent().context("project store has no parent")?)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            stages.push(entry.path());
        }
    }
    assert_eq!(stages.len(), 1, "expected exactly one retained stage");
    assert_reaped(&options, &stages[0]).await
}

/// Active open before the store exists: an unsupported schema version.
#[tokio::test]
async fn active_open_validation_error_returns_only_after_its_server_is_reaped() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = options(&root, '5')?;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    sqlx::query("UPDATE kuru_schema SET version = 99")
        .execute(store.pool.as_ref())
        .await?;
    store.close().await?;
    let error = crate::test_support::spawn_gated_open(options.clone())
        .await
        .expect_err("an unsupported schema version cannot open memory");
    assert!(format!("{error:#}").contains("unsupported"));
    let directory = project_directory(&options.data_dir, &options.project_scope)?;
    assert_reaped(&options, &directory).await
}

/// Active open after the store exists: a malformed usage-ledger receipt
/// schema refuses the ledger once candidate recovery has released the
/// startup guard.
#[tokio::test]
async fn established_store_open_error_returns_only_after_its_server_is_reaped() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = options(&root, '6')?;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    let usage_pool = store
        .shared
        .usage_pool
        .lock()
        .expect("usage pool lock")
        .clone()
        .context("usage ledger pool is unavailable")?;
    sqlx::query("ALTER TABLE operations DROP COLUMN label")
        .execute(usage_pool.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'malformed usage receipt schema', '--author', ?)")
        .bind(AUTHOR)
        .fetch_all(usage_pool.as_ref())
        .await?;
    drop(usage_pool);
    store.close().await?;
    let error = crate::test_support::spawn_gated_open(options.clone())
        .await
        .expect_err("a malformed usage receipt schema cannot open writable memory");
    // The usage ledger's historical receipt query is the refusing step.
    assert!(format!("{error:#}").contains("\"label\""));
    let directory = project_directory(&options.data_dir, &options.project_scope)?;
    assert_reaped(&options, &directory).await
}

/// Active open of a current-schema store that fails its final validation: a
/// dirty main working set.
#[tokio::test]
async fn active_open_final_validation_error_returns_only_after_its_server_is_reaped() -> Result<()>
{
    let root = crate::test_support::tempdir()?;
    let options = options(&root, '7')?;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    sqlx::query("CREATE TABLE uncommitted_fixture (id INT PRIMARY KEY)")
        .execute(store.pool.as_ref())
        .await?;
    store.close().await?;
    let error = crate::test_support::spawn_gated_open(options.clone())
        .await
        .expect_err("a dirty main working set cannot open writable memory");
    assert!(
        format!("{error:#}").contains("uncommitted changes"),
        "the open failed before its final validation: {error:#}"
    );
    let directory = project_directory(&options.data_dir, &options.project_scope)?;
    assert_reaped(&options, &directory).await
}

/// A close that fails after a failed open is attached to the open's error,
/// which stays the root cause; a clean close leaves the error unchanged.
#[test]
fn a_failed_close_is_attached_to_the_open_error() {
    let attached = with_close_failure(
        anyhow::anyhow!("open refused"),
        Err(anyhow::anyhow!("supervisor reap exceeded its allowance")),
    );
    assert_eq!(attached.root_cause().to_string(), "open refused");
    assert_eq!(
        format!("{attached:#}"),
        "memory server close after the failed open also failed: \
         supervisor reap exceeded its allowance: open refused"
    );

    let unchanged = with_close_failure(anyhow::anyhow!("open refused"), Ok(()));
    assert_eq!(format!("{unchanged:#}"), "open refused");
    assert_eq!(unchanged.chain().count(), 1);
}
