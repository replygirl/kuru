#![cfg(windows)]
use anyhow::{Context, Result, ensure};
use kuru_memory::{
    provision,
    server::{Server, ServerOptions},
    test_support,
};
use kuru_platform::{
    fs::{Directory, NameRetention, Privacy},
    windows::{
        pipe::{Pipe, PrivateListener},
        process::{Console, Lifetime, NativeChild, NativeSpawnSpec},
    },
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

async fn fixture(owner: &mut Fixture, binary: &Path, lifetime: Lifetime, mode: &str) -> Result<()> {
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
    owner.child = Some(command.spawn().await?);
    owner.channel = Some(
        listener
            .accept(owner.child.as_ref().unwrap(), Duration::from_secs(5))
            .await?,
    );
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

async fn loss(whole_job: bool, mode: &str) -> Result<()> {
    let mut owner = Fixture::new()?;
    let binary = provision::provision(&Default::default(), &test_support::cache_dir()).await?;
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
    let recovered = Server::open(opts).await?;
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
    let binary = provision::provision(&Default::default(), &test_support::cache_dir()).await?;
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
