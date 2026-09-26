#![cfg(windows)]
use anyhow::{Context, Result, ensure};
use kuru_memory::{
    server::{Server, ServerOptions},
    test_support,
};
use kuru_platform::{
    fs::{Directory, NameRetention, Privacy},
    windows::{
        pipe::{Pipe, PrivateListener},
        process::{Console, Lifetime, NativeChild, NativeSpawnSpec, Stdio},
    },
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn environment() -> Result<Vec<(std::ffi::OsString, std::ffi::OsString)>> {
    let system = kuru_platform::windows::process::system_directory()?;
    let root = system.parent().context("system directory has no parent")?;
    let mut environment = vec![
        ("SystemRoot".into(), root.as_os_str().into()),
        ("PATH".into(), system.into_os_string()),
    ];
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        environment.push(("LLVM_PROFILE_FILE".into(), profile));
    }
    Ok(environment)
}

fn options(root: &Path, binary: PathBuf) -> ServerOptions {
    ServerOptions {
        binary,
        directory: root.join("store café 東京"),
        project_scope: "native-owner-fixture".into(),
        supervisor: env!("CARGO_BIN_EXE_kuru-memory").into(),
        timeout: Duration::from_secs(25),
        read_only: false,
        retained: None,
        lifecycle_root: Some(root.join("lifecycles")),
    }
}

struct Fixture {
    root: Option<test_support::TempDir>,
    child: Option<NativeChild>,
    channel: Option<Pipe>,
    descendants_stopped: bool,
}

impl Fixture {
    fn new() -> Result<Self> {
        Ok(Self {
            root: Some(test_support::tempdir()?),
            child: None,
            channel: None,
            descendants_stopped: false,
        })
    }
    fn path(&self) -> &Path {
        self.root.as_ref().unwrap().path()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // EOF lets the actual creator perform its normal server close. Keep
        // its retained process handle even when a test assertion unwinds.
        drop(self.channel.take());
        let root = self.root.take().unwrap();
        let Some(mut child) = self.child.take() else {
            return;
        };
        let descendants_stopped = self.descendants_stopped;
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(40);
            let mut warned = false;
            loop {
                let status = child.try_wait();
                if matches!(status, Ok(Some(_))) {
                    break;
                }
                if !warned && (status.is_err() || std::time::Instant::now() >= deadline) {
                    eprintln!(
                        "native memory fixture cleanup is delayed; retaining creator and {}",
                        root.path().display()
                    );
                    warned = true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if !descendants_stopped {
                // Creator exit alone is not proof about its trusted supervisor.
                // Keep failure evidence and data until separately reconciled.
                eprintln!(
                    "native memory fixture descendant cleanup is unproven; preserved {}",
                    root.path().display()
                );
                std::mem::forget(root);
            }
        });
    }
}

async fn spawn_fixture(
    owner: &mut Fixture,
    binary: &Path,
    lifetime: Lifetime,
    mode: &str,
) -> Result<()> {
    let root = owner.path().to_owned();
    let listener = PrivateListener::bind()?;
    let mut command = NativeSpawnSpec::new(
        env!("CARGO_BIN_EXE_kuru-memory-parent-fixture").into(),
        root.clone(),
    );
    command.args = vec![
        root.as_os_str().into(),
        binary.as_os_str().into(),
        listener.address().into(),
        mode.into(),
    ];
    command.environment = environment()?;
    command.lifetime = lifetime;
    command.console = Console::PrivateHidden;
    if mode == "unconfigured" {
        command.stdout = Stdio::Pipe;
    }
    owner.child = Some(command.spawn().await?);
    owner.channel = Some(
        listener
            .accept(owner.child.as_ref().unwrap(), Duration::from_secs(5))
            .await?,
    );
    Ok(())
}

async fn fixture(owner: &mut Fixture, binary: &Path, lifetime: Lifetime, mode: &str) -> Result<()> {
    spawn_fixture(owner, binary, lifetime, mode).await?;
    let mut ready = [0; 6];
    tokio::time::timeout(
        Duration::from_secs(30),
        owner.channel.as_mut().unwrap().read_exact(&mut ready),
    )
    .await??;
    ensure!(
        &ready == b"READY\n",
        "memory owner fixture did not finish its accepted operation"
    );
    Ok(())
}

fn marker_seed(options: &kuru_memory::OpenOptions) -> Result<Vec<u8>> {
    Directory::ensure_private(&options.data_dir)?;
    let path = options.data_dir.join("memory.sqlite3");
    let source = rusqlite::Connection::open(&path)?;
    source.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA user_version=1; CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL); CREATE TABLE state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);")?;
    source.pragma_update(None, "application_id", 0x4b55_5255_i64)?;
    let namespace = format!("{}/transcript", options.project_scope);
    for (sequence, role, content) in [
        (7, "user", "accepted before marker"),
        (11, "assistant", "preserved imported reply"),
    ] {
        source.execute(
            "INSERT INTO messages VALUES (?1,?2,?3,?4)",
            rusqlite::params![sequence, namespace, role, content],
        )?;
    }
    source.execute(
        "INSERT INTO messages VALUES (19,'project/other/transcript','user','unrelated original')",
        [],
    )?;
    source.execute(
        "INSERT INTO state VALUES (?1,'\"jungian\"')",
        [format!("{}/framework", options.project_scope)],
    )?;
    source.execute(
        "INSERT INTO state VALUES ('project/other/framework','\"kept\"')",
        [],
    )?;
    source.close().map_err(|(_, error)| error)?;
    Ok(std::fs::read(path)?)
}

async fn marker_rows(
    options: &kuru_memory::OpenOptions,
    binary: &Path,
    directory: &Path,
    expected_revision: &str,
) -> Result<()> {
    let server = Server::open(ServerOptions {
        binary: binary.to_owned(),
        directory: directory.to_owned(),
        project_scope: options.project_scope.clone(),
        supervisor: options
            .supervisor
            .clone()
            .context("marker fixture has no supervisor")?,
        timeout: Duration::from_secs(25),
        read_only: true,
        retained: None,
        lifecycle_root: Some(options.data_dir.join("memory/lifecycles")),
    })
    .await?;
    let pool = server.pool("main").await?;
    let checked = async {
        let rows = sqlx::query_as::<_, (i64, Vec<u8>, Vec<u8>, String)>(
            "SELECT sequence, namespace, role, content FROM messages ORDER BY sequence",
        )
        .fetch_all(pool.as_ref())
        .await?;
        let namespace = format!("{}/transcript", options.project_scope).into_bytes();
        ensure!(
            rows == [
                (
                    7,
                    namespace.clone(),
                    b"user".to_vec(),
                    "accepted before marker".into()
                ),
                (
                    11,
                    namespace,
                    b"assistant".to_vec(),
                    "preserved imported reply".into()
                ),
            ],
            "marker recovery changed accepted rows or imported unrelated data"
        );
        let state =
            sqlx::query_as::<_, (Vec<u8>, String)>("SELECT `key`, value FROM state ORDER BY `key`")
                .fetch_all(pool.as_ref())
                .await?;
        ensure!(
            state
                == [(
                    format!("{}/framework", options.project_scope).into_bytes(),
                    "\"jungian\"".into()
                )],
            "marker recovery changed the accepted preference"
        );
        let revision: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
            .fetch_one(pool.as_ref())
            .await?;
        ensure!(
            revision == expected_revision,
            "marker recovery replayed a committed revision"
        );
        for message in [
            "Initialize Kuru memory schema 1",
            "Import preserved SQLite %",
        ] {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log WHERE message LIKE ?")
                    .bind(message)
                    .fetch_one(pool.as_ref())
                    .await?;
            ensure!(
                count == 1,
                "marker recovery created duplicate schema/import history"
            );
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    pool.close().await;
    let stopped = server.close().await;
    checked?;
    stopped
}

async fn marker_interruption(after_marker: bool, kill_creator: bool) -> Result<()> {
    use sha2::{Digest, Sha256};
    let mut owner = Fixture::new()?;
    let binary = test_support::warm_runtime_cache().await?;
    let options = test_support::windows::ready_marker_options(
        owner.path(),
        binary.clone(),
        env!("CARGO_BIN_EXE_kuru-memory-parent-fixture").into(),
    );
    let source = marker_seed(&options)?;
    let unrelated = options.data_dir.join("memory/unrelated retained café 東京");
    let unrelated_directory = Directory::ensure_private(&unrelated)?;
    let unrelated_identity = unrelated_directory.identity();
    {
        use std::io::Write;
        let mut file = unrelated_directory.create_new(std::ffi::OsStr::new("evidence"))?;
        file.write_all(b"unrelated retained bytes")?;
        file.sync_all()?;
    }
    let mode = if after_marker {
        "marker-after"
    } else {
        "marker-before"
    };
    spawn_fixture(&mut owner, &binary, Lifetime::TrustedSupervisor, mode).await?;
    let observed: test_support::windows::ReadyMarkerObservation =
        tokio::time::timeout(Duration::from_secs(60), async {
            let channel = owner.channel.as_mut().unwrap();
            let length = channel.read_u32().await? as usize;
            ensure!(
                length > 0 && length <= 16 * 1024,
                "invalid marker observation frame length"
            );
            let mut bytes = vec![0; length];
            channel.read_exact(&mut bytes).await?;
            let message: test_support::windows::ReadyMarkerMessage =
                serde_json::from_slice(&bytes)?;
            message
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("{mode} failed before its ready-marker observation"))
        })
        .await??;
    assert_eq!(observed.after_marker, after_marker);
    let active = options.data_dir.join("memory").join("6".repeat(64));
    assert!(!active.exists());
    assert_eq!(observed.stage.parent(), active.parent());
    assert!(
        observed
            .stage
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(&format!("{}.staging-", "6".repeat(64)))
    );
    let stage = Directory::open(&observed.stage, Privacy::OwnerOnly, NameRetention::Movable)?;
    assert_eq!(stage.identity().to_bytes(), observed.identity);
    let marker = if after_marker {
        let bytes = std::fs::read(observed.stage.join("ready.json"))?;
        let record: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(record["format"], 1);
        assert_eq!(record["project_scope"], options.project_scope);
        assert_eq!(record["initial_revision"], observed.initial_revision);
        assert_eq!(record["migration"]["messages"], 2);
        assert_eq!(record["migration"]["state"], 1);
        assert_eq!(record["migration"]["project_scope"], options.project_scope);
        let snapshot = PathBuf::from(
            record["migration"]["snapshot"]
                .as_str()
                .context("missing preserved snapshot")?,
        );
        assert_eq!(
            snapshot.parent(),
            Some(options.data_dir.join("memory/legacy").as_path())
        );
        let digest: String = Sha256::digest(std::fs::read(snapshot)?)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(record["migration"]["source_sha256"], digest);
        Some(bytes)
    } else {
        assert!(!observed.stage.join("ready.json").exists());
        None
    };
    let locks = Directory::open(
        &options.data_dir.join("memory/lifecycles"),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )?;
    let key: String = observed
        .identity
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let lock = locks.lock_file(std::ffi::OsStr::new(&format!("{key}.lock")))?;
    assert!(matches!(
        lock.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    ensure!(
        owner.child.as_mut().unwrap().try_wait()?.is_none(),
        "marker creator exited before the observed interruption"
    );
    if kill_creator {
        owner.child.as_mut().unwrap().terminate()?;
    } else {
        owner
            .channel
            .as_mut()
            .unwrap()
            .close(Duration::from_secs(3))
            .await?;
    }
    let status = owner
        .child
        .as_mut()
        .unwrap()
        .wait(Duration::from_secs(15))
        .await?;
    assert!(
        !status.success(),
        "interrupted marker creator unexpectedly completed activation"
    );
    owner
        .channel
        .as_mut()
        .unwrap()
        .close(Duration::from_secs(3))
        .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "marker supervisor did not finish owned database cleanup"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    assert!(!observed.stage.join("endpoint.json").exists());
    drop(lock);
    drop(stage);
    let recovered = kuru_memory::MemoryStore::open(options.clone()).await?;
    let revision = recovered.revision().await?;
    let preserved = options
        .data_dir
        .join("memory/interrupted")
        .join(observed.stage.file_name().unwrap());
    if after_marker {
        assert_eq!(revision, observed.initial_revision);
        assert_eq!(
            Directory::open(&active, Privacy::OwnerOnly, NameRetention::Movable)?
                .identity()
                .to_bytes(),
            observed.identity
        );
        assert_eq!(std::fs::read(active.join("ready.json"))?, marker.unwrap());
        assert!(!preserved.exists());
    } else {
        assert_ne!(
            Directory::open(&active, Privacy::OwnerOnly, NameRetention::Movable)?
                .identity()
                .to_bytes(),
            observed.identity
        );
        assert_eq!(
            Directory::open(&preserved, Privacy::OwnerOnly, NameRetention::Movable)?
                .identity()
                .to_bytes(),
            observed.identity
        );
        assert!(!preserved.join("ready.json").exists());
        marker_rows(&options, &binary, &preserved, &observed.initial_revision).await?;
    }
    assert!(!observed.stage.exists());
    recovered.close().await?;
    marker_rows(&options, &binary, &active, &revision).await?;
    let reopened = kuru_memory::MemoryStore::open(options.clone()).await?;
    assert_eq!(reopened.revision().await?, revision);
    assert_eq!(
        reopened
            .history(&format!("{}/transcript", options.project_scope), 20)
            .await?
            .len(),
        2
    );
    assert_eq!(
        reopened
            .get(&format!("{}/framework", options.project_scope))
            .await?,
        Some(serde_json::json!("jungian"))
    );
    reopened.close().await?;
    assert_eq!(
        std::fs::read(options.data_dir.join("memory.sqlite3"))?,
        source
    );
    assert_eq!(
        Directory::open(&unrelated, Privacy::OwnerOnly, NameRetention::Movable)?.identity(),
        unrelated_identity
    );
    assert_eq!(
        std::fs::read(unrelated.join("evidence"))?,
        b"unrelated retained bytes"
    );
    owner.descendants_stopped = true;
    Ok(())
}

#[tokio::test]
async fn creator_loss_before_ready_marker_preserves_committed_stage_and_imports_live_once()
-> Result<()> {
    marker_interruption(false, true).await
}

#[tokio::test]
async fn creator_loss_after_ready_marker_reuses_the_exact_committed_physical_stage() -> Result<()> {
    marker_interruption(true, true).await
}

#[tokio::test]
async fn marker_observer_eof_awaits_cleanup_without_stranding_the_release_channel() -> Result<()> {
    for after_marker in [false, true] {
        marker_interruption(after_marker, false).await?;
    }
    Ok(())
}

#[tokio::test]
async fn marker_startup_failure_reports_its_actual_cause_and_releases_the_writer() -> Result<()> {
    let mut owner = Fixture::new()?;
    let binary = test_support::warm_runtime_cache().await?;
    let options = test_support::windows::ready_marker_options(
        owner.path(),
        binary.clone(),
        env!("CARGO_BIN_EXE_kuru-memory-parent-fixture").into(),
    );
    marker_seed(&options)?;
    let source = options.data_dir.join("memory.sqlite3");
    let database = rusqlite::Connection::open(&source)?;
    database.pragma_update(None, "user_version", 99)?;
    database.close().map_err(|(_, error)| error)?;
    let rejected = std::fs::read(&source)?;
    spawn_fixture(
        &mut owner,
        &binary,
        Lifetime::TrustedSupervisor,
        "marker-before",
    )
    .await?;
    let message: test_support::windows::ReadyMarkerMessage =
        tokio::time::timeout(Duration::from_secs(30), async {
            let channel = owner.channel.as_mut().unwrap();
            let length = channel.read_u32().await? as usize;
            ensure!(
                length > 0 && length <= 16 * 1024,
                "invalid startup failure frame"
            );
            let mut bytes = vec![0; length];
            channel.read_exact(&mut bytes).await?;
            Ok::<_, anyhow::Error>(serde_json::from_slice(&bytes)?)
        })
        .await??;
    let error = message.unwrap_err();
    assert!(
        error.contains("unsupported legacy memory schema version 99"),
        "{error}"
    );
    assert!(
        !owner
            .child
            .as_mut()
            .unwrap()
            .wait(Duration::from_secs(15))
            .await?
            .success()
    );
    owner
        .channel
        .as_mut()
        .unwrap()
        .close(Duration::from_secs(3))
        .await?;
    assert_eq!(std::fs::read(&source)?, rejected);
    let database = rusqlite::Connection::open(&source)?;
    database.pragma_update(None, "user_version", 1)?;
    database.close().map_err(|(_, error)| error)?;
    let recovered = kuru_memory::MemoryStore::open(options).await?;
    assert_eq!(
        recovered
            .history(&format!("project/{}/transcript", "6".repeat(64)), 20)
            .await?
            .len(),
        2
    );
    recovered.close().await?;
    owner.descendants_stopped = true;
    Ok(())
}

async fn loss(whole_job: bool, mode: &str) -> Result<()> {
    let mut owner = Fixture::new()?;
    let binary = test_support::warm_runtime_cache().await?;
    let lifetime = if whole_job {
        Lifetime::OwnedJob
    } else {
        Lifetime::TrustedSupervisor
    };
    fixture(&mut owner, &binary, lifetime, mode).await?;
    let opts = options(owner.path(), binary);
    let store = Directory::open(&opts.directory, Privacy::OwnerOnly, NameRetention::Movable)?;
    let locks = Directory::open(
        opts.lifecycle_root.as_ref().unwrap(),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )?;
    let key: String = store
        .identity()
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let lock = locks.lock_file(std::ffi::OsStr::new(&format!("{key}.lock")))?;
    assert!(matches!(
        lock.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    let mut contender = opts.clone();
    contender.timeout = Duration::from_millis(200);
    assert!(
        Server::open(contender).await.is_err(),
        "a second writable server overlapped the retained owner"
    );
    assert!(matches!(
        lock.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    // This is a retained process handle or authoritative owned Job, never a PID.
    owner.child.as_mut().unwrap().terminate()?;
    owner
        .child
        .as_mut()
        .unwrap()
        .wait(Duration::from_secs(15))
        .await?;
    owner
        .channel
        .as_mut()
        .unwrap()
        .close(Duration::from_secs(3))
        .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "memory supervisor did not finish owned Dolt cleanup"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    if !whole_job {
        assert!(
            !opts.directory.join("endpoint.json").exists(),
            "creator-only EOF must perform normal endpoint retirement"
        );
    }
    drop(lock);
    let mut reset_listener = if whole_job {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint_path = opts.directory.join("endpoint.json");
        let mut endpoint: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&endpoint_path)?)?;
        let object = endpoint
            .as_object_mut()
            .context("published endpoint record is not an object")?;
        ensure!(
            object
                .get("instance")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "published endpoint record has no instance"
        );
        ensure!(
            object
                .get("port")
                .and_then(serde_json::Value::as_u64)
                .is_some(),
            "published endpoint record has no port"
        );
        object.insert(
            "port".into(),
            serde_json::Value::from(listener.local_addr()?.port()),
        );
        std::fs::write(&endpoint_path, serde_json::to_vec(&endpoint)?)?;
        Some(tokio::spawn(async move {
            let (probe, _) = listener.accept().await?;
            drop(probe);
            let (stream, _) = listener.accept().await?;
            stream.set_zero_linger()?;
            drop(stream);
            Ok::<_, std::io::Error>(())
        }))
    } else {
        None
    };
    let recovered = Server::open(opts).await;
    if let Some(listener) = reset_listener.as_mut() {
        match tokio::time::timeout(Duration::from_secs(3), &mut *listener).await {
            Ok(result) => result??,
            Err(error) => {
                listener.abort();
                let _ = listener.await;
                return Err(error)
                    .context("reset listener did not observe the raw probe and SQL connection");
            }
        }
    }
    let recovered = recovered?;
    let pool = recovered.pool("main").await?;
    if mode == "partial-ready" {
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT project_scope FROM kuru_instance WHERE singleton=1"
            )
            .fetch_one(pool.as_ref())
            .await?,
            "native-owner-fixture"
        );
    } else {
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT value FROM owner_receipt WHERE id=1")
                .fetch_one(pool.as_ref())
                .await?,
            "accepted before creator loss"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM owner_receipt WHERE id=2")
                .fetch_one(pool.as_ref())
                .await?,
            0,
            "the unfinished accepted transaction reached durable live state"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM dolt_log WHERE message='owner fixture accepted receipt'",
            )
            .fetch_one(pool.as_ref())
            .await?,
            1,
            "creator recovery replayed the accepted revision"
        );
    }
    pool.close().await;
    recovered.close().await?;
    owner.descendants_stopped = true;
    Ok(())
}

#[tokio::test]
async fn creator_only_loss_reaps_dolt_and_preserves_accepted_sql() -> Result<()> {
    loss(false, "committed").await
}

#[tokio::test]
async fn enclosing_job_loss_contains_the_tree_and_reopens_committed_state() -> Result<()> {
    loss(true, "committed").await
}

#[tokio::test]
async fn creator_loss_during_observed_inflight_sql_preserves_only_the_committed_revision()
-> Result<()> {
    loss(false, "inflight").await
}

#[tokio::test]
async fn creator_loss_after_partial_readiness_consumption_reaps_the_initialized_database()
-> Result<()> {
    loss(false, "partial-ready").await
}

#[tokio::test]
async fn normal_headless_dolt_close_reaps_the_supervisor_and_reopens_accepted_sql() -> Result<()> {
    let mut owner = Fixture::new()?;
    let binary = test_support::warm_runtime_cache().await?;
    fixture(
        &mut owner,
        &binary,
        Lifetime::TrustedSupervisor,
        "committed",
    )
    .await?;
    owner.channel.as_mut().unwrap().write_all(b"C").await?;
    owner.channel.as_mut().unwrap().flush().await?;
    let status = owner
        .child
        .as_mut()
        .unwrap()
        .wait(Duration::from_secs(15))
        .await?;
    assert!(
        status.success(),
        "real Dolt normal close did not complete: {status}"
    );
    let opts = options(owner.path(), binary);
    assert!(!opts.directory.join("endpoint.json").exists());
    let log = std::fs::read_to_string(opts.directory.join("server.log"))?;
    assert!(
        log.ends_with("\nKuru engine shutdown: Graceful\n"),
        "real Dolt did not complete the graceful shutdown branch: {log}"
    );
    let reopened = Server::open(opts).await?;
    let pool = reopened.pool("main").await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM owner_receipt WHERE id=1")
            .fetch_one(pool.as_ref())
            .await?,
        "accepted before creator loss"
    );
    pool.close().await;
    reopened.close().await?;
    owner.descendants_stopped = true;
    Ok(())
}

#[tokio::test]
async fn actual_engine_adapter_escalates_after_observed_break_and_reaps_locked_descendant()
-> Result<()> {
    let mut owner = Fixture::new()?;
    fixture(
        &mut owner,
        Path::new(env!("CARGO_BIN_EXE_kuru-memory-parent-fixture")),
        Lifetime::OwnedJob,
        "escalation",
    )
    .await?;
    let status = owner
        .child
        .as_mut()
        .unwrap()
        .wait(Duration::from_secs(8))
        .await?;
    assert!(
        status.success(),
        "engine adapter escalation fixture failed: {status}"
    );
    let directory = Directory::open(owner.path(), Privacy::OwnerOnly, NameRetention::Movable)?;
    let lock = directory.lock_file(std::ffi::OsStr::new("descendant.lock"))?;
    lock.try_lock()?;
    owner.descendants_stopped = true;
    Ok(())
}

#[tokio::test]
async fn retained_creator_loss_before_configuration_exits_without_database_or_lease() -> Result<()>
{
    let mut owner = Fixture::new()?;
    fixture(
        &mut owner,
        Path::new(env!("CARGO_BIN_EXE_kuru-memory-parent-fixture")),
        Lifetime::TrustedSupervisor,
        "unconfigured",
    )
    .await?;
    let child = owner.child.as_mut().unwrap();
    let mut output = child
        .take_stdout()
        .context("unconfigured fixture output missing")?;
    ensure!(
        child.try_wait()?.is_none(),
        "creator exited before the kill"
    );
    child.terminate()?;
    assert!(!child.wait(Duration::from_secs(8)).await?.success());
    owner
        .channel
        .as_mut()
        .unwrap()
        .close(Duration::from_secs(3))
        .await?;
    let mut completion = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(8),
        (&mut output).take(1024).read_to_end(&mut completion),
    )
    .await??;
    assert_eq!(completion, b"SUPERVISOR-EOF\n");
    output.close(Duration::from_secs(3)).await?;
    assert_eq!(
        std::fs::read_dir(owner.path())?.count(),
        0,
        "a supervisor without configuration created database or lease state"
    );
    owner.descendants_stopped = true;
    Ok(())
}

#[tokio::test]
async fn no_configuration_and_partial_frame_eof_exit_without_starting_a_database() -> Result<()> {
    for partial in [false, true] {
        let mut owner = Fixture::new()?;
        let listener = PrivateListener::bind()?;
        let mut command = NativeSpawnSpec::new(
            env!("CARGO_BIN_EXE_kuru-memory").into(),
            owner.path().into(),
        );
        command.args = vec![
            "--internal-dolt-supervisor".into(),
            listener.address().into(),
        ];
        command.environment = environment()?;
        command.lifetime = Lifetime::TrustedSupervisor;
        command.console = Console::PrivateHidden;
        owner.child = Some(command.spawn().await?);
        owner.channel = Some(
            listener
                .accept(owner.child.as_ref().unwrap(), Duration::from_secs(5))
                .await?,
        );
        let channel = owner.channel.as_mut().unwrap();
        if partial {
            channel.write_all(&[0, 0, 4, 0, b'{']).await?;
            channel.flush().await?;
            channel.close(Duration::from_secs(3)).await?;
        }
        let status = owner
            .child
            .as_mut()
            .unwrap()
            .wait(Duration::from_secs(8))
            .await?;
        assert!(
            !status.success(),
            "unconfigured supervisor unexpectedly succeeded"
        );
        channel.close(Duration::from_secs(3)).await?;
        assert_eq!(
            std::fs::read_dir(owner.path())?.count(),
            0,
            "unconfigured supervisor created memory state"
        );
        owner.descendants_stopped = true;
    }
    Ok(())
}
