//! Real-server recovery faults. Packet fixtures never log authentication payloads.
use super::*;
#[cfg(windows)]
use kuru_platform::windows::process::{
    Console, Lifetime, NativeChild, NativeSpawnSpec, Stdio as NativeStdio,
};
use serde_json::json;
#[cfg(unix)]
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(unix, windows))]
use std::{
    io::{Read, Write},
    path::PathBuf,
};
#[cfg(unix)]
use tokio::process::Command;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::{JoinHandle, JoinSet},
};

const TEST_DEADLINE: Duration = Duration::from_secs(10);
const FRAME_LIMIT: usize = 128 * 1024;

struct AbortUpgradeOnDrop(JoinHandle<Result<()>>);

impl Drop for AbortUpgradeOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[cfg(any(unix, windows))]
const PROCESS_OUTPUT_LIMIT: usize = 8 * 1024;

#[cfg(any(unix, windows))]
const PROCESS_LOSS_ROOT: &str = "KURU_MIGRATION_PROCESS_LOSS_ROOT";
#[cfg(any(unix, windows))]
const PROCESS_LOSS_SCOPE: &str = "KURU_MIGRATION_PROCESS_LOSS_SCOPE";
#[cfg(any(unix, windows))]
const PROCESS_LOSS_READY: &str = "KURU_MIGRATION_PROCESS_LOSS_READY";
#[cfg(any(unix, windows))]
const PROCESS_LOSS_STAGE: &str = "KURU_MIGRATION_PROCESS_LOSS_STAGE";
#[cfg(any(unix, windows))]
const PROCESS_LOSS_TEST: &str =
    "store::recovery_tests::process_loss_after_accepted_ddl_retains_attempt_until_cold_recovery";
#[cfg(any(unix, windows))]
const PROCESS_LOSS_STAGE_TEST: &str =
    "store::recovery_tests::fresh_staging_process_loss_after_ddl_is_preserved_and_never_reused";

fn migration_observation_deadline(options: &OpenOptions) -> Duration {
    Duration::from_secs(options.config.startup_timeout_secs).saturating_add(QUERY_TIMEOUT)
}

#[cfg(any(unix, windows))]
#[derive(Clone, Copy)]
enum ProcessLossMode {
    Existing,
    Staging,
}

#[cfg(unix)]
type ProcessLossNativeChild = tokio::process::Child;
#[cfg(windows)]
type ProcessLossNativeChild = NativeChild;
#[cfg(unix)]
type ProcessLossOutput = tokio::process::ChildStdout;
#[cfg(windows)]
type ProcessLossOutput = kuru_platform::windows::pipe::Pipe;

#[cfg(any(unix, windows))]
struct ProcessLossChild {
    child: Option<ProcessLossNativeChild>,
    reader: Option<JoinHandle<Result<Vec<u8>>>>,
    ready: Option<oneshot::Receiver<()>>,
    stderr: PathBuf,
    active_directory: PathBuf,
    quiescence_directory: Option<PathBuf>,
    lifecycle_root: Option<PathBuf>,
    readiness_deadline: Duration,
    retained_root: Option<Arc<crate::test_support::TempDir>>,
    descendants_quiescent: bool,
}

#[cfg(unix)]
async fn spawn_process_loss_creator(
    root: &std::path::Path,
    scope: &str,
    cache_dir: &std::path::Path,
    stderr: &std::path::Path,
    test_name: &'static str,
    mode: ProcessLossMode,
) -> Result<ProcessLossNativeChild> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(PROCESS_LOSS_ROOT, root)
        .env(PROCESS_LOSS_SCOPE, scope)
        // The child's environment is otherwise empty: it must receive the
        // parent's already-resolved engine cache directory explicitly rather
        // than re-deriving one, or its own KURU_DOLT_CACHE fallback can pick a
        // different (and on some platforms unwritable) directory.
        .env("KURU_DOLT_CACHE", cache_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(stderr)?)
        .kill_on_drop(true);
    if matches!(mode, ProcessLossMode::Staging) {
        command.env(PROCESS_LOSS_STAGE, "1");
    }
    for name in ["KURU_TEST_SUPERVISOR_PREPARED", "LLVM_PROFILE_FILE"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    // Held across the spawn; see `crate::spawn_gate`.
    let _gate = crate::spawn_gate::spawning().await;
    Ok(command.spawn()?)
}

#[cfg(windows)]
async fn spawn_process_loss_creator(
    root: &std::path::Path,
    scope: &str,
    cache_dir: &std::path::Path,
    stderr: &std::path::Path,
    test_name: &'static str,
    mode: ProcessLossMode,
) -> Result<ProcessLossNativeChild> {
    let executable = std::env::current_exe()?;
    let system = kuru_platform::windows::process::system_directory()?;
    let windows = system
        .parent()
        .context("Windows system directory has no parent")?;
    let mut command = NativeSpawnSpec::new(executable, root.to_owned());
    command.args = ["--exact", test_name, "--nocapture", "--test-threads=1"]
        .into_iter()
        .map(Into::into)
        .collect();
    command.environment = vec![
        ("SystemRoot".into(), windows.as_os_str().into()),
        ("PATH".into(), system.into_os_string()),
        (PROCESS_LOSS_ROOT.into(), root.as_os_str().into()),
        (PROCESS_LOSS_SCOPE.into(), scope.into()),
        // The child's environment is otherwise empty (no TMP/TEMP), so
        // std::env::temp_dir() inside it falls back to the Windows directory.
        // Pass the parent's already-resolved engine cache directory through
        // explicitly instead of relying on the child re-deriving one.
        ("KURU_DOLT_CACHE".into(), cache_dir.as_os_str().into()),
    ];
    if matches!(mode, ProcessLossMode::Staging) {
        command
            .environment
            .push((PROCESS_LOSS_STAGE.into(), "1".into()));
    }
    for name in ["KURU_TEST_SUPERVISOR_PREPARED", "LLVM_PROFILE_FILE"] {
        if let Some(value) = std::env::var_os(name) {
            command.environment.push((name.into(), value));
        }
    }
    let stderr: std::os::windows::io::OwnedHandle = std::fs::File::create(stderr)?.into();
    command.stdin = NativeStdio::Null;
    command.stdout = NativeStdio::Pipe;
    command.stderr = NativeStdio::Handle(stderr);
    command.lifetime = Lifetime::TrustedSupervisor;
    command.console = Console::PrivateHidden;
    // Held across the spawn; see `crate::spawn_gate`.
    let _gate = crate::spawn_gate::spawning().await;
    Ok(command.spawn().await?)
}

#[cfg(unix)]
fn take_process_loss_stdout(child: &mut ProcessLossNativeChild) -> Option<ProcessLossOutput> {
    child.stdout.take()
}

#[cfg(windows)]
fn take_process_loss_stdout(child: &mut ProcessLossNativeChild) -> Option<ProcessLossOutput> {
    child.take_stdout()
}

#[cfg(any(unix, windows))]
impl ProcessLossChild {
    async fn spawn(
        retained_root: Arc<crate::test_support::TempDir>,
        options: &OpenOptions,
        stderr: PathBuf,
        test_name: &'static str,
        mode: ProcessLossMode,
    ) -> Result<Self> {
        ensure!(
            options.data_dir == retained_root.path(),
            "process-loss retained root does not own the configured data directory"
        );
        let active_directory = project_directory(&options.data_dir, &options.project_scope)?;
        // The parent allowance also covers child initialization before the
        // child's own observation begins.
        let readiness_deadline = migration_observation_deadline(options)
            .saturating_add(Duration::from_secs(options.config.startup_timeout_secs));
        let cache_dir = options
            .config
            .cache_dir
            .as_deref()
            .context("process-loss fixture requires a resolved engine cache directory")?;
        let child = spawn_process_loss_creator(
            retained_root.path(),
            &options.project_scope,
            cache_dir,
            &stderr,
            test_name,
            mode,
        )
        .await?;
        let mut owned = Self {
            child: Some(child),
            reader: None,
            ready: None,
            stderr,
            active_directory: active_directory.clone(),
            quiescence_directory: matches!(mode, ProcessLossMode::Existing)
                .then_some(active_directory),
            lifecycle_root: cfg!(windows).then(|| options.data_dir.join("memory/lifecycles")),
            readiness_deadline,
            retained_root: Some(retained_root),
            descendants_quiescent: false,
        };
        let output = owned
            .child
            .as_mut()
            .and_then(take_process_loss_stdout)
            .context("process-loss child stdout missing");
        let output = match output {
            Ok(output) => output,
            Err(error) => return owned.finish(Err(error)).await,
        };
        let (send, receive) = oneshot::channel();
        owned.reader = Some(tokio::spawn(read_process_output(output, send)));
        owned.ready = Some(receive);
        Ok(owned)
    }

    async fn ready(&mut self) -> Result<()> {
        let ready = self
            .ready
            .take()
            .context("process-loss readiness was already observed")?;
        tokio::time::timeout(self.readiness_deadline, ready)
            .await
            .context("process-loss child readiness deadline exceeded")?
            .context("process-loss child did not reach accepted DDL")
    }

    fn set_staging_quiescence_directory(&mut self, stage: PathBuf) -> Result<()> {
        let parent = self
            .active_directory
            .parent()
            .context("process-loss active directory has no parent")?;
        ensure!(
            stage.parent() == Some(parent),
            "process-loss staging directory is outside its memory parent"
        );
        let active = self
            .active_directory
            .file_name()
            .context("process-loss active directory has no name")?
            .to_string_lossy();
        let name = stage
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .context("process-loss staging directory name is invalid")?;
        let suffix = name
            .strip_prefix(&format!("{active}.staging-"))
            .context("process-loss staging directory has an unexpected name")?;
        ensure!(
            Uuid::parse_str(suffix).is_ok() && stage.is_dir(),
            "process-loss staging directory is not a recognized live stage"
        );
        self.quiescence_directory = Some(stage);
        Ok(())
    }

    async fn assert_descendant_lifecycle_held(&self) -> Result<()> {
        let directory = self
            .quiescence_directory
            .as_deref()
            .context("process-loss cleanup target was not identified")?;
        let error = Server::quiescence_at(
            directory,
            self.lifecycle_root.as_deref(),
            Duration::from_millis(1),
        )
        .await
        .expect_err("process-loss creator did not retain its lifecycle lease");
        ensure!(
            format!("{error:#}").contains("memory lifecycle remains active"),
            "unexpected process-loss lifecycle observation: {error:#}"
        );
        Ok(())
    }

    async fn take_quiescence(&mut self) -> Result<LifecycleLease> {
        let directory = self
            .quiescence_directory
            .clone()
            .context("process-loss cleanup target was not identified")?;
        let lease =
            Server::quiescence_at(&directory, self.lifecycle_root.as_deref(), TEST_DEADLINE)
                .await?;
        self.descendants_quiescent = self.child.is_none();
        Ok(lease)
    }

    async fn finish<T>(mut self, observation: Result<T>) -> Result<T> {
        let process_cleanup = self.stop_and_reap().await;
        let quiescence = self.take_quiescence().await;
        let cleanup = match (process_cleanup, quiescence) {
            (Ok(output), Ok(lease)) => {
                drop(lease);
                Ok(output)
            }
            (Err(error), Ok(lease)) => {
                drop(lease);
                Err(error)
            }
            (Ok(_), Err(error)) => Err(error
                .context("process-loss descendants did not reach quiescence after child cleanup")),
            (Err(error), Err(quiescence)) => Err(error.context(format!(
                "process-loss descendant quiescence also failed: {quiescence:#}"
            ))),
        };
        let diagnostics = format!(
            "process-loss child stdout: {}; stderr: {}",
            cleanup
                .as_ref()
                .map(|bytes| String::from_utf8_lossy(bytes))
                .unwrap_or_else(|_| "<unavailable>".into()),
            bounded_fixture_text(&self.stderr)
        );
        match (observation, cleanup) {
            (Ok(value), Ok(_)) => Ok(value),
            (Err(error), Ok(_)) => Err(error.context(diagnostics)),
            (Ok(_), Err(cleanup)) => Err(cleanup.context(diagnostics)),
            (Err(error), Err(cleanup)) => Err(error.context(format!(
                "process-loss child cleanup also failed: {cleanup:#}; {diagnostics}"
            ))),
        }
    }

    async fn stop_and_reap(&mut self) -> Result<Vec<u8>> {
        let child_cleanup = async {
            let child = self.child.as_mut().context("process-loss child missing")?;
            #[cfg(unix)]
            {
                if child.try_wait()?.is_none() {
                    let _ = child.start_kill();
                }
                let waited = tokio::time::timeout(TEST_DEADLINE, child.wait())
                    .await
                    .context("process-loss child did not exit by the cleanup deadline")?;
                waited.context("wait for process-loss child")?;
            }
            #[cfg(windows)]
            {
                if child.try_wait()?.is_none() {
                    let _ = child.terminate();
                }
                child
                    .wait(TEST_DEADLINE)
                    .await
                    .context("process-loss child did not exit by the cleanup deadline")?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if child_cleanup.is_ok() {
            self.child.take();
        }

        let reader_cleanup = match self.reader.take() {
            Some(mut reader) => match tokio::time::timeout(TEST_DEADLINE, &mut reader).await {
                Ok(result) => result
                    .context("process-loss stdout reader task failed")
                    .and_then(|result| result),
                Err(_) => {
                    reader.abort();
                    let _ = reader.await;
                    Err(anyhow::anyhow!(
                        "process-loss stdout reader did not exit by the cleanup deadline"
                    ))
                }
            },
            None => Err(anyhow::anyhow!("process-loss stdout reader missing")),
        };
        match (child_cleanup, reader_cleanup) {
            (Ok(()), Ok(output)) => Ok(output),
            (Err(error), Ok(_)) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(reader)) => Err(error.context(format!(
                "process-loss stdout cleanup also failed: {reader:#}"
            ))),
        }
    }
}

#[cfg(any(unix, windows))]
impl Drop for ProcessLossChild {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
        if let Some(child) = self.child.as_mut() {
            #[cfg(unix)]
            let _ = child.start_kill();
            #[cfg(windows)]
            let _ = child.terminate();
        }
        if self.child.is_some() || !self.descendants_quiescent {
            // Cleanup uncertainty must not delete a directory that a surviving
            // supervisor or Dolt process can still hold. This remains true
            // after the creator process has been reaped but before its owned
            // descendants have released the lifecycle lease.
            if let Some(root) = self.retained_root.take() {
                std::mem::forget(root);
            }
        }
    }
}

#[cfg(any(unix, windows))]
async fn read_process_output<R>(mut output: R, ready: oneshot::Sender<()>) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let marker = PROCESS_LOSS_READY.as_bytes();
    let mut ready = Some(ready);
    let mut captured = Vec::with_capacity(PROCESS_OUTPUT_LIMIT);
    let mut tail = Vec::with_capacity(marker.len().saturating_sub(1));
    let mut chunk = [0_u8; 1024];
    loop {
        let count = output.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        let bytes = &chunk[..count];
        let available = PROCESS_OUTPUT_LIMIT.saturating_sub(captured.len());
        captured.extend_from_slice(&bytes[..bytes.len().min(available)]);

        tail.extend_from_slice(bytes);
        if tail.windows(marker.len()).any(|window| window == marker)
            && let Some(ready) = ready.take()
        {
            let _ = ready.send(());
        }
        let retained = marker.len().saturating_sub(1).min(tail.len());
        tail.drain(..tail.len() - retained);
    }
    Ok(captured)
}

#[cfg(any(unix, windows))]
fn bounded_fixture_text(path: &std::path::Path) -> String {
    let mut bytes = Vec::with_capacity(PROCESS_OUTPUT_LIMIT);
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file
            .take(PROCESS_OUTPUT_LIMIT as u64)
            .read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(any(unix, windows))]
async fn paused_process_loss_child(root: &std::path::Path, scope: String) -> Result<()> {
    let mut options = crate::test_support::open_options(root.to_owned(), scope)?;
    let observation_deadline = migration_observation_deadline(&options);
    let (hooks, control) =
        migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::AfterDdl);
    options.migration_hooks = Some(Arc::new(hooks));
    let mut opening = tokio::spawn(crate::test_support::spawn_gated_open(options));
    observe_process_loss_ddl(&mut opening, &control, observation_deadline).await?;
    println!("{PROCESS_LOSS_READY}");
    std::io::stdout().flush()?;
    std::future::pending::<()>().await;
    Ok(())
}

#[cfg(any(unix, windows))]
async fn observe_process_loss_ddl(
    opening: &mut JoinHandle<Result<MemoryStore>>,
    control: &migrations::MigrationPauseControl,
    deadline: Duration,
) -> Result<()> {
    let completed = tokio::time::timeout(deadline, async {
        tokio::select! {
            biased;
            opened = &mut *opening => match opened {
                Ok(Ok(store)) => Ok(Some(store)),
                Ok(Err(error)) => Err(error.context(
                    "process-loss child open failed before accepted DDL",
                )),
                Err(error) => Err(anyhow::Error::new(error).context(
                    "process-loss child open task failed before accepted DDL",
                )),
            },
            reached = control.reached() => reached
                .context("process-loss accepted-DDL observation failed")
                .map(|()| None),
        }
    })
    .await
    .context("process-loss child did not reach accepted DDL")??;
    let Some(store) = completed else {
        return Ok(());
    };

    let cleanup = match tokio::time::timeout(TEST_DEADLINE, store.close()).await {
        Ok(Ok(())) => "confirmed".to_owned(),
        Ok(Err(error)) => format!("failed: {error:#}"),
        Err(_) => "timed out".to_owned(),
    };
    anyhow::bail!(
        "process-loss child open completed before accepted DDL; returned store cleanup {cleanup}"
    )
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn process_loss_observer_surfaces_open_error_before_ddl_deadline() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = OpenOptions::new(
        root.path().to_owned(),
        format!("project/{}", "e".repeat(64)),
    );
    options.config.startup_timeout_secs = 0;
    let (hooks, control) =
        migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::AfterDdl);
    options.migration_hooks = Some(Arc::new(hooks));
    let mut opening = tokio::spawn(crate::test_support::spawn_gated_open(options));

    let error = observe_process_loss_ddl(&mut opening, &control, TEST_DEADLINE)
        .await
        .expect_err("invalid open must fail before reaching accepted DDL");
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("process-loss child open failed before accepted DDL"),
        "missing observer context: {rendered}"
    );
    assert!(
        rendered.contains("memory.startup_timeout_secs must be between 1 and 300"),
        "missing original open error: {rendered}"
    );
    assert!(
        !rendered.contains("deadline has elapsed"),
        "real open error was masked by the observer deadline: {rendered}"
    );
    Ok(())
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn process_loss_after_accepted_ddl_retains_attempt_until_cold_recovery() -> Result<()> {
    if let Some(root) = std::env::var_os(PROCESS_LOSS_ROOT) {
        let scope =
            std::env::var(PROCESS_LOSS_SCOPE).context("process-loss child scope missing")?;
        return paused_process_loss_child(std::path::Path::new(&root), scope).await;
    }

    let root = Arc::new(crate::test_support::tempdir()?);
    let options = crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", "e".repeat(64)),
    )?;
    super::tests::released_v1(&options).await?;
    let server = super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
        .bind(b"process-loss-source".as_slice())
        .bind("\"retained\"")
        .execute(main.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'process-loss source', '--author', ?)")
        .bind(AUTHOR)
        .fetch_all(main.as_ref())
        .await?;
    let base = revision(&main).await?;
    let candidate = format!("candidate_{}", Uuid::new_v4().simple());
    sqlx::query("CALL DOLT_BRANCH(?, ?)")
        .bind(&candidate)
        .bind(&base)
        .fetch_all(main.as_ref())
        .await?;
    let candidate_pool = server.pool(&candidate).await?;
    sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
        .bind(b"process-loss-candidate".as_slice())
        .bind(b"assistant".as_slice())
        .bind("candidate retained")
        .execute(candidate_pool.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'process-loss candidate', '--author', ?)")
        .bind(AUTHOR)
        .fetch_all(candidate_pool.as_ref())
        .await?;
    let candidate_head = revision(&candidate_pool).await?;
    candidate_pool.close().await;
    main.close().await;
    server.close().await?;

    let stderr = root.path().join("process-loss-child.stderr");
    let mut child = ProcessLossChild::spawn(
        root.clone(),
        &options,
        stderr,
        PROCESS_LOSS_TEST,
        ProcessLossMode::Existing,
    )
    .await?;
    child.ready().await?;
    assert_startup_lock_held(&options)?;
    child.assert_descendant_lifecycle_held().await?;
    let (hooks, control) =
        migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::BeforeBranch);
    let mut recovery_options = options.clone();
    recovery_options.migration_hooks = Some(Arc::new(hooks));
    let mut recovering = Box::pin(crate::test_support::spawn_gated_open(recovery_options));
    std::future::poll_fn(|context| {
        std::task::Poll::Ready(
            match std::future::Future::poll(recovering.as_mut(), context) {
                std::task::Poll::Pending => Ok(()),
                std::task::Poll::Ready(Ok(_)) => Err(anyhow::anyhow!(
                    "production contender crossed the live creator's startup ownership"
                )),
                std::task::Poll::Ready(Err(error)) => {
                    Err(error.context("production contender failed during its first poll"))
                }
            },
        )
    })
    .await?;
    child.assert_descendant_lifecycle_held().await?;
    let recovering = tokio::spawn(recovering);
    let _creator_output = child
        .stop_and_reap()
        .await
        .context("terminate and reap process-loss creator")?;
    // This boundary is inside the same production open future that was pending
    // against the live creator. Reaching it requires taking the returned startup
    // guard after the orphaned supervisor has completed its reap.
    tokio::time::timeout(TEST_DEADLINE, control.reached())
        .await
        .context("production contender did not take over after creator cleanup")??;
    assert_startup_lock_held(&options)?;

    // The contender is now the lifecycle owner but is paused before its first
    // migration mutation. Inspect through the actual read-only attach path.
    let inspection = Server::open(ServerOptions {
        binary: provision::provision(&options.config, &options.data_dir.join("tools/dolt")).await?,
        directory: project_directory(&options.data_dir, &options.project_scope)?,
        project_scope: options.project_scope.clone(),
        supervisor: options
            .supervisor
            .clone()
            .context("process-loss inspection supervisor missing")?,
        timeout: Duration::from_secs(options.config.startup_timeout_secs),
        read_only: true,
        retained: None,
        lifecycle_root: cfg!(windows).then(|| options.data_dir.join("memory/lifecycles")),
    })
    .await?;
    let inspected_state = async {
        let inspected = inspection.pool("main").await?;
        assert_eq!(revision(&inspected).await?, base);
        assert_eq!(migrations::version(&inspected).await?, 1);
        assert_clean_status(inspected.as_ref()).await;
        let failed_branch: String = sqlx::query_scalar(
            "SELECT name FROM dolt_branches WHERE LEFT(BINARY name, 15) = BINARY 'kuru_migration_'",
        )
        .fetch_one(inspected.as_ref())
        .await?;
        let failed = inspection.pool(&failed_branch).await?;
        let failed_head = revision(&failed).await?;
        assert_eq!(failed_head, base, "failed DDL must remain outside HEAD");
        assert_eq!(migrations::version(&failed).await?, 1);
        let failed_status = working_status(&failed).await;
        assert_eq!(
            failed_status,
            vec![("kuru_migrations".into(), 0, "new table".into())],
            "accepted DDL must retain its exact bounded working-set evidence"
        );
        failed.close().await;
        let inspected_candidate = inspection.pool(&candidate).await?;
        assert_eq!(revision(&inspected_candidate).await?, candidate_head);
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT 1"
            )
            .bind(b"process-loss-candidate".as_slice())
            .fetch_one(inspected_candidate.as_ref())
            .await?,
            "candidate retained"
        );
        inspected_candidate.close().await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_status")
                .fetch_one(inspected.as_ref())
                .await?,
            0
        );
        inspected.close().await;
        Ok::<_, anyhow::Error>((failed_branch, failed_head, failed_status))
    }
    .await;
    let inspection_cleanup = inspection.close().await;
    control.resume();
    let recovered = tokio::time::timeout(TEST_DEADLINE, recovering)
        .await
        .context("cold recovery did not finish")?
        .context("cold recovery worker failed")??;
    let (failed_branch, failed_head, failed_status) = match (inspected_state, inspection_cleanup) {
        (Ok(state), Ok(())) => state,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup)) => {
            return Err(error.context(format!(
                "process-loss inspection cleanup also failed: {cleanup:#}"
            )));
        }
    };
    assert_eq!(
        migrations::version(&recovered.pool).await?,
        migrations::CURRENT_VERSION
    );
    assert_eq!(
        recovered.get("process-loss-source").await?,
        Some(json!("retained"))
    );
    assert_clean_status(recovered.pool.as_ref()).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kuru_migrations")
            .fetch_one(recovered.pool.as_ref())
            .await?,
        i64::from(migrations::CURRENT_VERSION - 1)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_branches WHERE LEFT(BINARY name, 15) = BINARY 'kuru_migration_'")
            .fetch_one(recovered.pool.as_ref())
            .await?,
        4,
        "cold recovery retains the failed branch and all three ordered migration attempts"
    );
    let retained_failed = recovered.shared.server.pool(&failed_branch).await?;
    assert_eq!(revision(&retained_failed).await?, failed_head);
    assert_eq!(migrations::version(&retained_failed).await?, 1);
    assert_eq!(working_status(&retained_failed).await, failed_status);
    retained_failed.close().await;
    let recovered_head = recovered.revision().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'"
        )
        .fetch_one(recovered.pool.as_ref())
        .await?,
        1,
        "cold recovery must publish the v2 migration exactly once"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 3%'"
        )
        .fetch_one(recovered.pool.as_ref())
        .await?,
        1,
        "cold recovery must publish the v3 migration exactly once"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 4%'"
        )
        .fetch_one(recovered.pool.as_ref())
        .await?,
        1,
        "cold recovery must publish the v4 migration exactly once"
    );
    let recovered_candidate = recovered.shared.server.pool(&candidate).await?;
    assert_eq!(revision(&recovered_candidate).await?, candidate_head);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT 1"
        )
        .bind(b"process-loss-candidate".as_slice())
        .fetch_one(recovered_candidate.as_ref())
        .await?,
        "candidate retained"
    );
    recovered_candidate.close().await;
    recovered.close().await?;
    let reopened = tokio::time::timeout(
        TEST_DEADLINE,
        crate::test_support::spawn_gated_open(options),
    )
    .await??;
    assert_eq!(reopened.revision().await?, recovered_head);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'"
        )
        .fetch_one(reopened.pool.as_ref())
        .await?,
        1,
        "stopped reopen must not add migration work"
    );
    reopened.close().await?;
    let released_lifecycle = child.take_quiescence().await?;
    drop(released_lifecycle);
    Ok(())
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn fresh_staging_process_loss_after_ddl_is_preserved_and_never_reused() -> Result<()> {
    if let Some(root) = std::env::var_os(PROCESS_LOSS_ROOT) {
        ensure!(
            std::env::var_os(PROCESS_LOSS_STAGE).as_deref() == Some(std::ffi::OsStr::new("1")),
            "fresh staging child missing its process-loss mode"
        );
        let scope =
            std::env::var(PROCESS_LOSS_SCOPE).context("process-loss child scope missing")?;
        return paused_process_loss_child(std::path::Path::new(&root), scope).await;
    }
    let root = Arc::new(crate::test_support::tempdir()?);
    let options = crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", "f".repeat(64)),
    )?;
    let active = project_directory(&options.data_dir, &options.project_scope)?;
    let parent = active
        .parent()
        .context("fresh stage parent missing")?
        .to_owned();
    let stderr = root.path().join("fresh-staging-process-loss.stderr");
    let mut child = ProcessLossChild::spawn(
        root.clone(),
        &options,
        stderr,
        PROCESS_LOSS_STAGE_TEST,
        ProcessLossMode::Staging,
    )
    .await?;
    let observation = async {
        child.ready().await?;
        let mut stages = std::fs::read_dir(&parent)?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(".staging-"))
            })
            .collect::<Vec<_>>();
        ensure!(
            stages.len() == 1,
            "fresh process-loss fixture requires one live staging directory"
        );
        let stage = stages.pop().expect("one stage checked");
        ensure!(
            !active.exists() && !active.join("ready.json").exists(),
            "fresh DDL must not activate the project"
        );
        ensure!(
            !stage.join("ready.json").exists(),
            "fresh DDL stage must remain markerless"
        );
        assert_startup_lock_held(&options)?;
        child.set_staging_quiescence_directory(stage.clone())?;
        let live_directory = files::directory(&stage)?;
        Ok::<_, anyhow::Error>((stage, live_directory))
    }
    .await;
    let (stage, stage_directory) = child.finish(observation).await?;
    ensure!(
        stage.exists() && !stage.join("ready.json").exists(),
        "stopped staging identity must remain before cold recovery"
    );
    let stage_identity = stage_directory.identity();
    let before = observe_stopped_stage(&options, &stage).await?;
    assert_eq!(before.main_version, 1);
    assert!(before.main_status.is_empty());
    assert_eq!(before.attempt_head, before.main_head);
    assert_eq!(before.attempt_version, 1);
    assert_eq!(
        before.attempt_status,
        vec![("kuru_migrations".into(), 0, "new table".into())]
    );

    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    assert_eq!(
        migrations::version(&store.pool).await?,
        migrations::CURRENT_VERSION
    );
    assert_clean_status(store.pool.as_ref()).await;
    store.close().await?;
    ensure!(
        active.join("ready.json").is_file(),
        "cold open must create a separate active store"
    );
    let interrupted = parent
        .join("interrupted")
        .join(stage.file_name().context("stage name missing")?);
    ensure!(
        interrupted.is_dir(),
        "cold recovery must preserve the markerless dirty stage"
    );
    ensure!(
        !interrupted.join("ready.json").exists(),
        "preserved stage must remain markerless"
    );
    let interrupted_directory = files::directory(&interrupted)?;
    assert_eq!(stage_directory.identity(), stage_identity);
    assert_eq!(interrupted_directory.identity(), stage_identity);
    assert_ne!(files::directory(&active)?.identity(), stage_identity);
    assert_eq!(observe_stopped_stage(&options, &interrupted).await?, before);
    Ok(())
}

#[cfg(any(unix, windows))]
#[derive(Debug, Eq, PartialEq)]
struct StoppedStageSnapshot {
    main_head: String,
    main_version: i32,
    main_status: Vec<(String, i64, String)>,
    attempt_head: String,
    attempt_name: String,
    attempt_version: i32,
    attempt_status: Vec<(String, i64, String)>,
}

#[cfg(any(unix, windows))]
async fn observe_stopped_stage(
    options: &OpenOptions,
    directory: &std::path::Path,
) -> Result<StoppedStageSnapshot> {
    let binary =
        provision::provision(&options.config, &options.data_dir.join("tools/dolt")).await?;
    let server = Server::open(ServerOptions {
        binary,
        directory: directory.to_owned(),
        project_scope: options.project_scope.clone(),
        supervisor: options
            .supervisor
            .clone()
            .context("stage fixture supervisor missing")?,
        timeout: Duration::from_secs(options.config.startup_timeout_secs),
        read_only: true,
        retained: None,
        lifecycle_root: cfg!(windows).then(|| options.data_dir.join("memory/lifecycles")),
    })
    .await?;
    let observed = async {
        let main = server.pool("main").await?;
        let branches: Vec<String> = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar(
                "SELECT name FROM dolt_branches WHERE LEFT(BINARY name, 15) = BINARY 'kuru_migration_' LIMIT 2",
            )
            .fetch_all(main.as_ref()),
        )
        .await
        .context("stopped-stage attempt inventory deadline exceeded")??;
        ensure!(
            branches.len() == 1,
            "stopped stage must retain exactly one migration attempt"
        );
        let branch = branches.into_iter().next().expect("one branch checked");
        let failed = server.pool(&branch).await?;
        Ok::<_, anyhow::Error>(StoppedStageSnapshot {
            main_head: revision(&main).await?,
            main_version: migrations::version(&main).await?,
            main_status: checked_working_status(&main).await?,
            attempt_head: revision(&failed).await?,
            attempt_name: branch,
            attempt_version: migrations::version(&failed).await?,
            attempt_status: checked_working_status(&failed).await?,
        })
    }
    .await;
    let closed = server.close().await;
    match (observed, closed) {
        (Ok(observed), Ok(())) => Ok(observed),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(error.context(format!(
            "stopped-stage inspection cleanup also failed: {cleanup:#}"
        ))),
    }
}

#[cfg(any(unix, windows))]
async fn checked_working_status(pool: &MySqlPool) -> Result<Vec<(String, i64, String)>> {
    let rows = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query(
            "SELECT table_name, staged, status FROM dolt_status ORDER BY BINARY table_name, staged, BINARY status LIMIT 65",
        )
        .fetch_all(pool),
    )
    .await
    .context("stopped-stage status inventory deadline exceeded")??;
    ensure!(
        rows.len() <= 64,
        "stopped-stage status inventory is excessive"
    );
    rows.into_iter()
        .map(|row| {
            Ok((
                row.try_get("table_name")?,
                row.try_get("staged")?,
                row.try_get("status")?,
            ))
        })
        .collect()
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn process_loss_child_cleanup_runs_after_failed_observation() -> Result<()> {
    let root = Arc::new(crate::test_support::tempdir()?);
    let options = crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", "d".repeat(64)),
    )?;
    super::tests::released_v1(&options).await?;
    let server = super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let base = revision(&main).await?;
    main.close().await;
    server.close().await?;

    let stderr = root.path().join("process-loss-observation.stderr");
    let mut child = ProcessLossChild::spawn(
        root.clone(),
        &options,
        stderr,
        PROCESS_LOSS_TEST,
        ProcessLossMode::Existing,
    )
    .await?;
    let observation = child.ready().await.and_then(|()| {
        Err::<(), _>(anyhow::anyhow!(
            "controlled process-loss observation failure"
        ))
    });
    let error = child
        .finish(observation)
        .await
        .expect_err("controlled observation failure was lost");
    assert!(
        format!("{error:#}").contains("controlled process-loss observation failure"),
        "cleanup replaced the observation error: {error:#}"
    );

    let directory = project_directory(&options.data_dir, &options.project_scope)?;
    let lifecycle_root = cfg!(windows).then(|| options.data_dir.join("memory/lifecycles"));
    let lease = Server::quiescence_at(
        &directory,
        lifecycle_root.as_deref(),
        Duration::from_secs(1),
    )
    .await?;
    drop(lease);
    let inspection = super::tests::released_server(&options).await?;
    let main = inspection.pool("main").await?;
    assert_eq!(revision(&main).await?, base);
    assert_eq!(migrations::version(&main).await?, 1);
    assert_clean_status(main.as_ref()).await;
    let attempts: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM dolt_branches WHERE LEFT(BINARY name, 15) = BINARY 'kuru_migration_'",
    )
    .fetch_all(main.as_ref())
    .await?;
    assert_eq!(attempts.len(), 1, "accepted DDL attempt was not retained");
    let failed = inspection.pool(&attempts[0]).await?;
    assert_eq!(revision(&failed).await?, base);
    assert_eq!(migrations::version(&failed).await?, 1);
    assert_eq!(
        working_status(&failed).await,
        vec![("kuru_migrations".into(), 0, "new table".into())]
    );
    failed.close().await;
    main.close().await;
    inspection.close().await?;
    Ok(())
}

fn assert_startup_lock_held(options: &OpenOptions) -> Result<()> {
    let directory = project_directory(&options.data_dir, &options.project_scope)?;
    let parent = directory
        .parent()
        .context("fixture project path has no parent")?;
    let locks = Directory::open(
        &parent.join("locks"),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )?;
    let name = directory
        .file_name()
        .context("fixture project path has no name")?;
    let file = locks.lock_file(name)?;
    locks.verify(name, &file)?;
    ensure!(
        matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        "accepted migration worker released the checked startup lock"
    );
    Ok(())
}

#[tokio::test]
async fn cancelled_upgrade_call_retains_writer_through_accepted_ddl_boundaries() -> Result<()> {
    for boundary in [
        migrations::MigrationBoundary::BeforeBranch,
        migrations::MigrationBoundary::BeforeDdl,
        migrations::MigrationBoundary::AfterDdl,
        migrations::MigrationBoundary::BeforeCommit,
        migrations::MigrationBoundary::BeforePublish,
    ] {
        let root = crate::test_support::tempdir()?;
        let mut options = crate::test_support::open_options(
            root.path().to_owned(),
            format!("project/{}", Uuid::new_v4().simple().to_string().repeat(2)),
        )?;
        super::tests::released_v1(&options).await?;
        let source = super::tests::released_server(&options).await?;
        let source_pool = source.pool("main").await?;
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(b"migration-source".as_slice())
            .bind("\"retained\"")
            .execute(source_pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'migration source', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(source_pool.as_ref())
            .await?;
        source_pool.close().await;
        source.close().await?;
        let completion_deadline = migration_observation_deadline(&options);
        let (hooks, control) = migrations::MigrationRunnerHooks::paused(boundary);
        options.migration_hooks = Some(Arc::new(hooks));
        let opening = tokio::spawn(crate::test_support::spawn_gated_open(options.clone()));
        tokio::time::timeout(TEST_DEADLINE, control.reached())
            .await
            .context("migration worker did not reach its accepted cancellation boundary")??;
        opening.abort();
        assert!(opening.await.unwrap_err().is_cancelled());
        assert_startup_lock_held(&options)?;

        assert!(
            tokio::time::timeout(
                Duration::from_millis(80),
                crate::test_support::spawn_gated_open(options.clone())
            )
            .await
            .is_err(),
            "a competing opener acquired writer authority before the accepted worker reaped"
        );
        control.resume();
        let store = tokio::time::timeout(
            completion_deadline,
            crate::test_support::spawn_gated_open(options),
        )
        .await
        .with_context(|| {
            format!(
                "accepted migration worker did not finish after caller cancellation at {boundary:?}"
            )
        })??;
        assert_eq!(
            migrations::version(&store.pool).await?,
            migrations::CURRENT_VERSION
        );
        assert_eq!(
            store.get("migration-source").await?,
            Some(json!("retained"))
        );
        assert_clean_status(store.pool.as_ref()).await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kuru_migrations")
                .fetch_one(store.pool.as_ref())
                .await?,
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM dolt_branches WHERE LEFT(BINARY name, 15) = BINARY 'kuru_migration_'"
            )
            .fetch_one(store.pool.as_ref())
            .await?,
            3
        );
        store.close().await?;
    }
    Ok(())
}

#[tokio::test]
async fn live_original_session_blocks_receipt_reconciliation_even_after_commit() {
    let store = MemoryStore::temporary().await.unwrap();
    let operation = Uuid::new_v4().to_string();
    let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
    apply(
        &mut connection,
        &operation,
        "barrier",
        Mutation::State(vec![("settled".into(), "true".into())]),
        None,
    )
    .await
    .unwrap();
    let committed_revision = store.revision().await.unwrap();
    assert!(operation_exists(&store.pool, &operation).await.unwrap());
    *store.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Operation(operation),
    });
    // Receipt existence alone must not clear an original session that could
    // still finish another part of the accepted operation.
    assert!(
        tokio::time::timeout(Duration::from_millis(80), store.reconcile())
            .await
            .is_err()
    );
    assert!(store.shared.uncertain.lock().unwrap().is_some());
    assert!(
        await_session_end(&store.pool, id, Duration::from_millis(30))
            .await
            .is_err()
    );
    drop(connection);
    tokio::time::timeout(TEST_DEADLINE, store.reconcile())
        .await
        .unwrap()
        .unwrap();
    assert!(store.shared.uncertain.lock().unwrap().is_none());
    assert_eq!(store.get("settled").await.unwrap(), Some(json!(true)));
    store.reconcile().await.unwrap();
    assert_eq!(store.revision().await.unwrap(), committed_revision);
    store.close().await.unwrap();
}

#[tokio::test]
async fn dropped_uncommitted_session_resolves_absent_receipt_only_after_teardown() {
    let store = MemoryStore::temporary().await.unwrap();
    let revision = store.revision().await.unwrap();
    let operation = Uuid::new_v4().to_string();
    let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
    sqlx::query("START TRANSACTION")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO state (`key`, value) VALUES ('uncommitted', 'true')")
        .execute(&mut connection)
        .await
        .unwrap();
    *store.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Operation(operation),
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), store.reconcile())
            .await
            .is_err()
    );
    assert!(store.shared.uncertain.lock().unwrap().is_some());
    drop(connection);
    assert_eq!(
        tokio::time::timeout(TEST_DEADLINE, store.resolve_uncertain())
            .await
            .unwrap()
            .unwrap(),
        Some(false)
    );
    assert!(store.get("uncommitted").await.unwrap().is_none());
    assert_eq!(store.revision().await.unwrap(), revision);
    store.put("after rollback", &json!(1)).await.unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn in_flight_disconnect_waits_for_real_query_and_session_teardown() -> Result<()> {
    let store = MemoryStore::temporary().await.unwrap();
    let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
    let running = tokio::spawn(async move {
        let _ = sqlx::query("SELECT SLEEP(5)")
            .execute(&mut connection)
            .await;
    });
    tokio::time::timeout(TEST_DEADLINE, async {
        loop {
            let query: Option<String> =
                sqlx::query_scalar("SELECT INFO FROM information_schema.processlist WHERE ID = ?")
                    .bind(id)
                    .fetch_optional(store.pool.as_ref())
                    .await
                    .unwrap();
            if query.is_some_and(|query| query.contains("SLEEP(5)")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        await_session_end(&store.pool, id, Duration::from_millis(30))
            .await
            .is_err()
    );
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    await_session_end(&store.pool, id, TEST_DEADLINE)
        .await
        .unwrap();
    // Independent authenticated readers must survive this owned socket close.
    assert_eq!(store.status().await.unwrap().engine, "dolt");
    let pool = store.pool.clone();
    let before = (pool.size(), pool.num_idle(), pool.is_closed());
    let close_started = std::time::Instant::now();
    if let Err(error) = store.close().await {
        let after = (pool.size(), pool.num_idle(), pool.is_closed());
        return Err(error.context(format!(
            "fixture pool close state: before size={} idle={} closed={}; after size={} idle={} closed={}; elapsed_ms={}",
            before.0,
            before.1,
            before.2,
            after.0,
            after.1,
            after.2,
            close_started.elapsed().as_millis(),
        )));
    }
    Ok(())
}

/// A slow pool drain does not make the exact owner reap disposable. Once the
/// the owned engine has reaped, a returned dead client connection must still
/// be drained before explicit close reports success.
#[cfg(unix)]
#[tokio::test]
async fn post_reap_pool_drain_finishes_a_returned_real_connection() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = crate::test_support::open_options(
        root.path().to_path_buf(),
        format!("project/{}", "a".repeat(64)),
    )?;
    let store = crate::test_support::spawn_gated_open(options).await?;
    let directory = store.shared.directory.clone();
    let pool = store.pool.clone();
    let held = pool.acquire().await?;
    let reaped = store.shared.server.observe_next_owner_reap().await?;
    let close = tokio::spawn(async move { store.close().await });

    // The held SQLx permit forces the first graceful drain to its deadline.
    // The test signal is sent only after the retained child actually reaps.
    // Its lifecycle lease may now be acquired by a successor, while explicit
    // close still owes completion of the already-closed client pool.
    tokio::time::timeout(Duration::from_secs(30), reaped)
        .await
        .context("owner did not reap while the pool connection was held")??;
    drop(held);
    tokio::time::timeout(Duration::from_secs(15), close)
        .await
        .context("post-reap pool drain did not finish")???;
    let lease = crate::server::Server::quiescence(&directory, Duration::from_secs(5)).await?;
    drop(lease);
    ensure!(
        pool.is_closed() && pool.size() == 0,
        "post-reap pool still retains a connection"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn post_reap_pool_drain_reports_a_connection_that_never_returns() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = crate::test_support::open_options(
        root.path().to_path_buf(),
        format!("project/{}", "b".repeat(64)),
    )?;
    let store = crate::test_support::spawn_gated_open(options).await?;
    let directory = store.shared.directory.clone();
    let pool = store.pool.clone();
    let held = pool.acquire().await?;
    let reaped = store.shared.server.observe_next_owner_reap().await?;
    let close = tokio::spawn(async move { store.close().await });

    tokio::time::timeout(Duration::from_secs(30), reaped)
        .await
        .context("owner did not reap before the second pool deadline")??;
    let error = tokio::time::timeout(Duration::from_secs(15), close)
        .await
        .context("persistently held connection exceeded the second shutdown deadline")??
        .unwrap_err();
    ensure!(
        error
            .to_string()
            .contains("post-reap memory pool close deadline exceeded"),
        "persistent pool connection did not retain an actionable error: {error:#}"
    );
    let lease = crate::server::Server::quiescence(&directory, Duration::from_secs(5)).await?;
    drop(lease);
    drop(held);
    tokio::time::timeout(Duration::from_secs(10), pool.close())
        .await
        .context("held pool connection did not drain after fixture release")?;
    ensure!(pool.size() == 0, "fixture pool retained a connection");
    Ok(())
}

#[tokio::test]
async fn lost_commit_reply_recovers_one_durable_update_and_reopens_without_replay() {
    let directory = crate::test_support::tempdir().unwrap();
    let options = crate::test_support::open_options(
        directory.path().to_path_buf(),
        format!("project/{}", "e".repeat(64)),
    )
    .unwrap();
    let store = crate::test_support::spawn_gated_open(options.clone())
        .await
        .unwrap();
    let before = store.revision().await.unwrap();
    let proxy = AckDropProxy::start(
        store.pool.clone(),
        "COMMIT",
        DurableObservation::RevisionAdvanced { base: before },
    )
    .await;
    let affected = proxy.view(&store).await;
    tokio::time::timeout(
        TEST_DEADLINE,
        affected.append("conversation", "user", "exactly once"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        proxy.discarded.load(Ordering::Acquire),
        "fixture must discard an actual durable COMMIT reply"
    );
    assert!(store.shared.uncertain.lock().unwrap().is_none());
    let revision = store.revision().await.unwrap();
    let operations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM operations")
        .fetch_one(store.pool.as_ref())
        .await
        .unwrap();
    assert_eq!(operations, 1);
    assert_eq!(store.history("conversation", 10).await.unwrap().len(), 1);
    affected.reconcile().await.unwrap();
    assert_eq!(store.revision().await.unwrap(), revision);
    affected.pool.close().await;
    proxy.close().await;
    store.close().await.unwrap();
    let reopened = crate::test_support::spawn_gated_open(options)
        .await
        .unwrap();
    assert_eq!(reopened.revision().await.unwrap(), revision);
    assert_eq!(
        reopened.history("conversation", 10).await.unwrap()[0].plain_text(),
        Some("exactly once")
    );
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn lost_promotion_reply_reconciles_target_once_and_keeps_later_writes() {
    let store = MemoryStore::temporary().await.unwrap();
    store.put("before", &json!(true)).await.unwrap();
    let candidate = store.begin_candidate("acknowledgement test").await.unwrap();
    candidate
        .view()
        .put("dream", &json!("accepted"))
        .await
        .unwrap();
    let target = candidate.view().revision().await.unwrap();
    let proxy = AckDropProxy::start(
        store.pool.clone(),
        "CALL DOLT_MERGE",
        DurableObservation::MainAtTarget {
            target: target.clone(),
        },
    )
    .await;
    let affected = proxy.view(&store).await;
    let candidate = Candidate {
        live: affected.clone(),
        view: candidate.view(),
        base: candidate.base,
        promoted: Arc::new(StdMutex::new(None)),
    };
    assert_eq!(
        tokio::time::timeout(TEST_DEADLINE, candidate.promote())
            .await
            .unwrap()
            .unwrap(),
        target
    );
    assert!(
        proxy.discarded.load(Ordering::Acquire),
        "fixture must discard a real completed merge reply"
    );
    assert!(store.shared.uncertain.lock().unwrap().is_none());
    assert_eq!(store.get("dream").await.unwrap(), Some(json!("accepted")));
    assert_eq!(candidate.promote().await.unwrap(), target);
    store.put("later", &json!("retained")).await.unwrap();
    let after = store.revision().await.unwrap();
    affected.reconcile().await.unwrap();
    assert_eq!(store.revision().await.unwrap(), after);
    assert_eq!(store.get("later").await.unwrap(), Some(json!("retained")));
    affected.pool.close().await;
    proxy.close().await;
    store.close().await.unwrap();
}

#[tokio::test]
async fn promotion_receipt_keeps_base_target_and_refuses_divergent_history() {
    let store = MemoryStore::temporary().await.unwrap();
    let candidate = store.begin_candidate("receipt barrier").await.unwrap();
    candidate.view().put("candidate", &json!(1)).await.unwrap();
    let target = candidate.view().revision().await.unwrap();
    let (connection, id) = owned_connection(&store.pool).await.unwrap();
    *store.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Promotion {
            base: candidate.base.clone(),
            target: target.clone(),
        },
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), store.reconcile())
            .await
            .is_err()
    );
    assert!(
        matches!(&store.shared.uncertain.lock().unwrap().as_ref().unwrap().receipt, Receipt::Promotion { base, target: pending } if base == candidate.base() && pending == &target)
    );
    drop(connection);
    assert_eq!(store.resolve_uncertain().await.unwrap(), Some(false));
    assert!(store.get("candidate").await.unwrap().is_none());
    store.put("live", &json!(2)).await.unwrap();
    *store.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Promotion {
            base: candidate.base,
            target,
        },
    });
    assert!(
        store
            .reconcile()
            .await
            .unwrap_err()
            .to_string()
            .contains("diverged")
    );
    assert!(store.shared.uncertain.lock().unwrap().is_some());
    assert!(store.put("must not write", &json!(3)).await.is_err());
    assert!(store.get("must not write").await.unwrap().is_none());
    store.close().await.unwrap();
}

const SCHEMA_BOUNDARY_TABLE: &str = "schema_transaction_probe";
const SCHEMA_BOUNDARY_NAMESPACE: &str = "schema-boundary-source";
const CANDIDATE_NAMESPACE: &str = "schema-boundary-candidate";
const V2_RECEIPT_ID: &str = "kuru.memory.receipts.v2";
const V2_RECEIPT_SQL: &str = "CREATE TABLE kuru_migrations (version INT PRIMARY KEY, id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, digest CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, operation CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL UNIQUE)";
const V2_RECEIPT_DIGEST: &str = "12ba323bce2fca30bd5a22ff0248adbb72fefb26f5bd5f7333e186a525f2979d";

/// These transaction-boundary probes must begin from the released v1 schema:
/// updating `kuru_schema` to v2 is part of the atomic DDL receipt they observe.
/// `MemoryStore::temporary` deliberately creates the current schema instead.
async fn stopped_released_v1_store() -> MemoryStore {
    let root = Arc::new(
        tempfile::Builder::new()
            .prefix("kuru-memory-")
            .tempdir()
            .unwrap(),
    );
    let data = root.path().join("private");
    let options =
        crate::test_support::open_options(data, format!("project/{}", "0".repeat(64))).unwrap();
    super::tests::released_v1(&options).await.unwrap();
    let directory = project_directory(&options.data_dir, &options.project_scope).unwrap();
    let server = Server::open(ServerOptions {
        binary: provision::provision(&options.config, &options.data_dir.join("tools/dolt"))
            .await
            .unwrap(),
        directory: directory.clone(),
        project_scope: options.project_scope.clone(),
        supervisor: options.supervisor.clone().unwrap(),
        timeout: Duration::from_secs(options.config.startup_timeout_secs),
        read_only: false,
        retained: Some(root),
        lifecycle_root: cfg!(windows).then(|| options.data_dir.join("memory/lifecycles")),
    })
    .await
    .unwrap();
    let pool = server.pool("main").await.unwrap();
    MemoryStore {
        shared: Arc::new(Shared {
            server,
            directory,
            project_scope: options.project_scope,
            read_only: false,
            write: Arc::new(Mutex::new(())),
            uncertain: StdMutex::new(None),
            usage_pool: StdMutex::new(None),
            candidate_recovery_pause: None,
            _permit: None,
        }),
        pool,
        branch: "main".into(),
        logical_receipt: None,
    }
}

struct CandidateSnapshot {
    view: MemoryStore,
    branch: String,
    revision: String,
    history: Vec<Message>,
}

async fn preserved_candidate(store: &MemoryStore) -> CandidateSnapshot {
    let candidate = store
        .begin_candidate("schema transaction boundary")
        .await
        .unwrap();
    let view = candidate.view().clone();
    view.append(
        CANDIDATE_NAMESPACE,
        "assistant",
        "candidate history retained",
    )
    .await
    .unwrap();
    let revision = view.revision().await.unwrap();
    let history = view.history(CANDIDATE_NAMESPACE, 10).await.unwrap();
    CandidateSnapshot {
        branch: view.branch.clone(),
        view,
        revision,
        history,
    }
}

async fn staged_schema_transaction(
    connection: &mut MySqlConnection,
    receipt: &str,
    commit: bool,
) -> Result<()> {
    sqlx::query("START TRANSACTION")
        .execute(&mut *connection)
        .await?;
    sqlx::query("CREATE TABLE schema_transaction_probe (id INT PRIMARY KEY, version INT NOT NULL)")
        .execute(&mut *connection)
        .await?;
    // This is an immutable copy of the current v2 registry definition. The
    // validator below fails if either the fixture or registry drifts.
    sqlx::query(V2_RECEIPT_SQL)
        .execute(&mut *connection)
        .await?;
    sqlx::query("UPDATE kuru_schema SET version = 2 WHERE id = 1")
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
        .bind(receipt)
        .bind("schema transaction receipt")
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO kuru_migrations (version, id, digest, operation) VALUES (2, ?, ?, ?)")
        .bind(V2_RECEIPT_ID)
        .bind(V2_RECEIPT_DIGEST)
        .bind(receipt)
        .execute(&mut *connection)
        .await?;
    if commit {
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
            .bind(format!("schema transaction boundary [{receipt}]"))
            .bind(AUTHOR)
            .fetch_all(&mut *connection)
            .await?;
        sqlx::query("COMMIT").execute(&mut *connection).await?;
    }
    Ok(())
}

struct SchemaBranch {
    view: MemoryStore,
    base: String,
}

async fn exact_base_schema_branch(store: &MemoryStore) -> SchemaBranch {
    let base = store.revision().await.unwrap();
    let branch = format!("migration_schema_{}", Uuid::new_v4().simple());
    sqlx::query("CALL DOLT_BRANCH(?, ?)")
        .bind(&branch)
        .bind(&base)
        .fetch_all(store.pool.as_ref())
        .await
        .unwrap();
    let pool = store.shared.server.pool(&branch).await.unwrap();
    SchemaBranch {
        view: MemoryStore {
            shared: store.shared.clone(),
            pool,
            branch,
            logical_receipt: None,
        },
        base,
    }
}

async fn assert_clean_status(pool: &MySqlPool) {
    let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(dirty, 0, "fixture must not leave working-set changes");
}

async fn assert_candidate_unchanged(candidate: &CandidateSnapshot) {
    let observed = observe_candidate(candidate).await;
    assert_eq!(observed.branch, candidate.branch, "{observed:?}");
    assert_eq!(observed.ref_hash, candidate.revision, "{observed:?}");
    assert_eq!(observed.history, candidate.history, "{observed:?}");
}

#[derive(Debug)]
struct CandidateObservation {
    branch: String,
    ref_hash: String,
    history: Vec<Message>,
}

async fn observe_candidate(candidate: &CandidateSnapshot) -> CandidateObservation {
    let branch: String = sqlx::query_scalar("SELECT name FROM dolt_branches WHERE name = ?")
        .bind(&candidate.branch)
        .fetch_one(candidate.view.pool.as_ref())
        .await
        .unwrap();
    let branch_ref: String = sqlx::query_scalar("SELECT hash FROM dolt_branches WHERE name = ?")
        .bind(&candidate.branch)
        .fetch_one(candidate.view.pool.as_ref())
        .await
        .unwrap();
    CandidateObservation {
        branch,
        ref_hash: branch_ref,
        history: candidate
            .view
            .history(CANDIDATE_NAMESPACE, 10)
            .await
            .unwrap(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ProbeAtHead {
    Rows(i64),
    Error(String),
}

async fn probe_table_at_head(pool: &MySqlPool) -> ProbeAtHead {
    match sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM schema_transaction_probe AS OF 'HEAD'")
        .fetch_one(pool)
        .await
    {
        Ok(rows) => ProbeAtHead::Rows(rows),
        Err(error) => ProbeAtHead::Error(error.to_string()),
    }
}

async fn assert_committed_schema(
    store: &MemoryStore,
    before: &str,
    source_history: &[Message],
    candidate: &CandidateSnapshot,
    receipt: &str,
) {
    assert_schema_commit(store, before, source_history, receipt).await;
    assert_candidate_unchanged(candidate).await;
}

async fn assert_schema_commit(
    view: &MemoryStore,
    before: &str,
    source_history: &[Message],
    receipt: &str,
) {
    assert_eq!(
        migrations::validate_historical(&view.pool).await.unwrap(),
        2,
        "the manual transaction must publish the exact released v2 schema"
    );
    let head = view.revision().await.unwrap();
    assert_ne!(head, before);
    let parent: String = sqlx::query_scalar(
        "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
    )
    .bind(&head)
    .fetch_one(view.pool.as_ref())
    .await
    .unwrap();
    assert_eq!(
        parent, before,
        "the old main HEAD must be the new commit parent"
    );
    assert_eq!(
        probe_table_at_head(view.pool.as_ref()).await,
        ProbeAtHead::Rows(0),
        "the committed schema table must be visible AS OF HEAD"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
            .fetch_one(view.pool.as_ref())
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT label FROM operations AS OF 'HEAD' WHERE id = ?")
            .bind(receipt)
            .fetch_one(view.pool.as_ref())
            .await
            .unwrap(),
        "schema transaction receipt"
    );
    let commits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log WHERE message = ?")
        .bind(format!("schema transaction boundary [{receipt}]"))
        .fetch_one(view.pool.as_ref())
        .await
        .unwrap();
    assert_eq!(commits, 1, "fixture must publish exactly one schema commit");
    assert_eq!(
        history_at_head(view.pool.as_ref(), SCHEMA_BOUNDARY_NAMESPACE).await,
        source_history
    );
    assert_clean_status(view.pool.as_ref()).await;
}

#[tokio::test]
async fn manual_dolt_commit_atomically_publishes_ddl_version_and_receipt() {
    let store = stopped_released_v1_store().await;
    store
        .append(SCHEMA_BOUNDARY_NAMESPACE, "user", "main history retained")
        .await
        .unwrap();
    let before = store.revision().await.unwrap();
    let source_history = history_at_head(store.pool.as_ref(), SCHEMA_BOUNDARY_NAMESPACE).await;
    let candidate = preserved_candidate(&store).await;
    let receipt = Uuid::new_v4().to_string();
    let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
    staged_schema_transaction(&mut connection, &receipt, true)
        .await
        .unwrap();
    drop(connection);
    await_session_end(&store.pool, id, TEST_DEADLINE)
        .await
        .unwrap();
    assert_committed_schema(&store, &before, &source_history, &candidate, &receipt).await;
    store.close().await.unwrap();
}

#[tokio::test]
async fn dropping_precommit_ddl_session_retains_dirty_working_ddl_outside_head() {
    let store = stopped_released_v1_store().await;
    store
        .append(SCHEMA_BOUNDARY_NAMESPACE, "user", "main history retained")
        .await
        .unwrap();
    let before = store.revision().await.unwrap();
    let source_history = history_at_head(store.pool.as_ref(), SCHEMA_BOUNDARY_NAMESPACE).await;
    let candidate = preserved_candidate(&store).await;
    let receipt = Uuid::new_v4().to_string();
    let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
    staged_schema_transaction(&mut connection, &receipt, false)
        .await
        .unwrap();
    drop(connection);
    await_session_end(&store.pool, id, TEST_DEADLINE)
        .await
        .unwrap();
    let observation = PrecommitObservation {
        before: before.clone(),
        after: store.revision().await.unwrap(),
        probe_at_head: probe_table_at_head(store.pool.as_ref()).await,
        version_at_head: sqlx::query_scalar(
            "SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1",
        )
        .fetch_one(store.pool.as_ref())
        .await
        .unwrap(),
        receipt_at_head: sqlx::query_scalar(
            "SELECT COUNT(*) FROM operations AS OF 'HEAD' WHERE id = ?",
        )
        .bind(&receipt)
        .fetch_one(store.pool.as_ref())
        .await
        .unwrap(),
        working_status: working_status(store.pool.as_ref()).await,
        candidate: observe_candidate(&candidate).await,
    };
    assert_eq!(observation.after, observation.before, "{observation:?}");
    assert!(
        matches!(&observation.probe_at_head, ProbeAtHead::Error(error) if error.contains(SCHEMA_BOUNDARY_TABLE)),
        "uncommitted schema must be absent AS OF HEAD: {observation:?}"
    );
    assert_eq!(
        observation.version_at_head, 1,
        "uncommitted schema-version update must be absent at HEAD: {observation:?}"
    );
    assert_eq!(
        observation.receipt_at_head, 0,
        "uncommitted receipt must be absent at HEAD: {observation:?}"
    );
    assert_eq!(
        observation.working_status,
        vec![
            ("kuru_migrations".into(), 0, "new table".into()),
            (SCHEMA_BOUNDARY_TABLE.into(), 0, "new table".into()),
        ],
        "pinned Dolt retains each uncommitted migration row outside HEAD: {observation:?}"
    );
    assert_eq!(
        history_at_head(store.pool.as_ref(), SCHEMA_BOUNDARY_NAMESPACE).await,
        source_history,
        "{observation:?}"
    );
    assert_eq!(
        observation.candidate.branch, candidate.branch,
        "{observation:?}"
    );
    assert_eq!(
        observation.candidate.ref_hash, candidate.revision,
        "{observation:?}"
    );
    assert_eq!(
        observation.candidate.history, candidate.history,
        "{observation:?}"
    );
    store.close().await.unwrap();
}

#[derive(Debug)]
struct PrecommitObservation {
    before: String,
    after: String,
    probe_at_head: ProbeAtHead,
    version_at_head: i64,
    receipt_at_head: i64,
    working_status: Vec<(String, i64, String)>,
    candidate: CandidateObservation,
}

async fn working_status(pool: &MySqlPool) -> Vec<(String, i64, String)> {
    sqlx::query("SELECT table_name, staged, status FROM dolt_status ORDER BY table_name")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            Ok::<_, sqlx::Error>((
                row.try_get("table_name")?,
                row.try_get("staged")?,
                row.try_get("status")?,
            ))
        })
        .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
        .unwrap()
}

async fn history_at_head(pool: &MySqlPool, namespace: &str) -> Vec<Message> {
    let version: i32 =
        sqlx::query_scalar("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
            .fetch_one(pool)
            .await
            .unwrap();
    let query = if version >= 3 {
        "SELECT role, content_format, content FROM messages AS OF 'HEAD' WHERE namespace = ? ORDER BY sequence"
    } else {
        "SELECT role, content FROM messages AS OF 'HEAD' WHERE namespace = ? ORDER BY sequence"
    };
    sqlx::query(query)
        .bind(namespace.as_bytes())
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let role = String::from_utf8(row.try_get::<Vec<u8>, _>("role")?)?;
            let content: String = row.try_get("content")?;
            let format: String = if version >= 3 {
                row.try_get("content_format")?
            } else {
                "text-v1".into()
            };
            super::decode_message(role, &format, &content)
        })
        .collect::<Result<Vec<_>>>()
        .unwrap()
}

async fn assert_dirty_schema_branch(
    branch: &SchemaBranch,
    receipt: &str,
    source_history: &[Message],
) {
    assert_eq!(branch.view.revision().await.unwrap(), branch.base);
    assert!(
        matches!(probe_table_at_head(branch.view.pool.as_ref()).await, ProbeAtHead::Error(error) if error.contains(SCHEMA_BOUNDARY_TABLE)),
        "failed branch must retain the DDL outside its HEAD"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
            .fetch_one(branch.view.pool.as_ref())
            .await
            .unwrap(),
        1,
        "failed branch must retain schema version 1 at HEAD"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM operations AS OF 'HEAD' WHERE id = ?")
            .bind(receipt)
            .fetch_one(branch.view.pool.as_ref())
            .await
            .unwrap(),
        0,
        "failed branch must retain no receipt at HEAD"
    );
    assert_eq!(
        working_status(branch.view.pool.as_ref()).await,
        vec![
            ("kuru_migrations".into(), 0, "new table".into()),
            (SCHEMA_BOUNDARY_TABLE.into(), 0, "new table".into()),
        ],
        "failed exact-base branch must preserve its dirty DDL"
    );
    assert_eq!(
        history_at_head(branch.view.pool.as_ref(), SCHEMA_BOUNDARY_NAMESPACE).await,
        source_history
    );
    assert!(
        branch
            .view
            .history(SCHEMA_BOUNDARY_NAMESPACE, 10)
            .await
            .is_err(),
        "the public historical reader must reject pending receipt authority"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT name FROM dolt_branches WHERE name = ?")
            .bind(&branch.view.branch)
            .fetch_one(branch.view.pool.as_ref())
            .await
            .unwrap(),
        branch.view.branch
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT hash FROM dolt_branches WHERE name = ?")
            .bind(&branch.view.branch)
            .fetch_one(branch.view.pool.as_ref())
            .await
            .unwrap(),
        branch.base
    );
}

#[tokio::test]
async fn lost_manual_dolt_commit_reply_reconciles_one_clean_schema_commit() {
    let store = stopped_released_v1_store().await;
    store
        .append(SCHEMA_BOUNDARY_NAMESPACE, "user", "main history retained")
        .await
        .unwrap();
    let before = store.revision().await.unwrap();
    let source_history = history_at_head(store.pool.as_ref(), SCHEMA_BOUNDARY_NAMESPACE).await;
    let candidate = preserved_candidate(&store).await;
    let receipt = Uuid::new_v4().to_string();
    let proxy = AckDropProxy::start(
        store.pool.clone(),
        "CALL DOLT_COMMIT",
        DurableObservation::RevisionAdvanced {
            base: before.clone(),
        },
    )
    .await;
    let affected = proxy.view(&store).await;
    let (mut connection, id) = owned_connection(&affected.pool).await.unwrap();
    let error = staged_schema_transaction(&mut connection, &receipt, true)
        .await
        .unwrap_err();
    assert!(
        proxy.discarded.load(Ordering::Acquire),
        "fixture must discard an actual durable DOLT_COMMIT reply: {error:#}"
    );
    drop(connection);
    await_session_end(&store.pool, id, TEST_DEADLINE)
        .await
        .unwrap();
    assert_committed_schema(&store, &before, &source_history, &candidate, &receipt).await;
    affected.pool.close().await;
    proxy.close().await;
    store.close().await.unwrap();
}

#[tokio::test]
async fn production_upgrade_reconciles_lost_commit_reply_after_routed_session_ends() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", "f".repeat(64)),
    )?;
    super::tests::released_v1(&options).await?;
    let server = super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let base = revision(&main).await?;
    main.close().await;
    server.close().await?;

    let reserved = ReservedAckDropProxy::reserve().await;
    let (hooks, control) =
        migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::BeforeCommit);
    let hooks = hooks.with_route(migrations::MigrationBoundary::BeforeCommit, reserved.port);
    let completion_deadline = migration_observation_deadline(&options);
    options.migration_hooks = Some(Arc::new(hooks));
    let opening = tokio::spawn(crate::test_support::spawn_gated_open(options.clone()));

    let source = tokio::time::timeout(TEST_DEADLINE, control.route_source())
        .await
        .context("production migration did not expose its routed fixture source")??;
    let proxy = reserved.start(
        source,
        "CALL DOLT_COMMIT",
        DurableObservation::RevisionAdvanced { base: base.clone() },
    );
    control.resume_route();
    tokio::time::timeout(TEST_DEADLINE, control.reached())
        .await
        .context("production migration did not reach the commit boundary")??;
    control.resume();

    let store = tokio::time::timeout(completion_deadline, opening)
        .await
        .with_context(|| {
            format!(
                "production migration did not reconcile the lost commit reply (reply_discarded={}, routed_session_ended={})",
                proxy.discarded.load(Ordering::Acquire),
                proxy.session_ended.load(Ordering::Acquire)
            )
        })???;
    assert!(
        proxy.discarded.load(Ordering::Acquire),
        "fixture must discard the durable production migration DOLT_COMMIT reply"
    );
    await_flag(&proxy.session_ended, TEST_DEADLINE)
        .await
        .context(
            "the proxy must observe the original routed SQL session end before reconciliation",
        )?;
    assert_eq!(
        migrations::version(&store.pool).await?,
        migrations::CURRENT_VERSION
    );
    assert_clean_status(store.pool.as_ref()).await;
    let upgraded = store.revision().await?;
    let commits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'",
    )
    .fetch_one(store.pool.as_ref())
    .await?;
    assert_eq!(commits, 1, "production migration must publish exactly once");
    store.close().await?;
    proxy.close().await;

    let reopened = tokio::time::timeout(
        TEST_DEADLINE,
        crate::test_support::spawn_gated_open(options),
    )
    .await
    .context("reopen replayed a migration hook instead of recognizing the durable upgrade")??;
    assert_eq!(reopened.revision().await?, upgraded);
    let commits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'",
    )
    .fetch_one(reopened.pool.as_ref())
    .await?;
    assert_eq!(commits, 1, "reopen must not replay a reconciled migration");
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn production_upgrade_reconciles_lost_branch_reply_after_exact_ref_creation() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", "b".repeat(64)),
    )?;
    super::tests::released_v1(&options).await?;
    let server = super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
        .bind(b"branch-reply-source".as_slice())
        .bind("\"retained\"")
        .execute(main.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'branch reply source', '--author', ?)")
        .bind(AUTHOR)
        .fetch_all(main.as_ref())
        .await?;
    let base = revision(&main).await?;
    main.close().await;
    server.close().await?;

    let reserved = ReservedAckDropProxy::reserve().await;
    let (hooks, control) =
        migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::BeforeBranch);
    let hooks = hooks.with_route(migrations::MigrationBoundary::BeforeBranch, reserved.port);
    options.migration_hooks = Some(Arc::new(hooks));
    let opening = tokio::spawn(crate::test_support::spawn_gated_open(options.clone()));
    let source = tokio::time::timeout(TEST_DEADLINE, control.route_source())
        .await
        .context("production migration did not expose branch-route source")??;
    let branch = control.branch_name()?;
    let proxy = reserved.start(
        source,
        "CALL DOLT_BRANCH",
        DurableObservation::BranchAtBase {
            branch: branch.clone(),
            base: base.clone(),
        },
    );
    control.resume_route();
    tokio::time::timeout(TEST_DEADLINE, control.reached())
        .await
        .context("production migration did not reach branch boundary")??;
    control.resume();

    let store = tokio::time::timeout(TEST_DEADLINE, opening)
        .await
        .context("production migration did not reconcile lost branch reply")???;
    assert!(proxy.discarded.load(Ordering::Acquire));
    await_flag(&proxy.session_ended, TEST_DEADLINE).await?;
    assert_eq!(
        store.get("branch-reply-source").await?,
        Some(json!("retained"))
    );
    assert_eq!(
        migrations::version(&store.pool).await?,
        migrations::CURRENT_VERSION
    );
    assert_clean_status(store.pool.as_ref()).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kuru_migrations")
            .fetch_one(store.pool.as_ref())
            .await?,
        i64::from(migrations::CURRENT_VERSION - 1)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'"
        )
        .fetch_one(store.pool.as_ref())
        .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_branches WHERE name = ?")
            .bind(&branch)
            .fetch_one(store.pool.as_ref())
            .await?,
        1
    );
    let upgraded = store.revision().await?;
    store.close().await?;
    proxy.close().await;
    let reopened = tokio::time::timeout(
        TEST_DEADLINE,
        crate::test_support::spawn_gated_open(options),
    )
    .await??;
    assert_eq!(reopened.revision().await?, upgraded);
    assert_eq!(
        reopened.get("branch-reply-source").await?,
        Some(json!("retained"))
    );
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn usage_upgrade_reconciles_lost_publication_reply_without_replacing_legacy_receipt()
-> Result<()> {
    usage_upgrade_publication_fixture(true).await
}

#[tokio::test]
async fn usage_upgrade_preserves_ready_attempt_when_publication_never_dispatches() -> Result<()> {
    usage_upgrade_publication_fixture(false).await
}

async fn usage_upgrade_publication_fixture(accepted: bool) -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", Uuid::new_v4().simple().to_string().repeat(2)),
    )?;
    super::tests::released_v1(&options).await?;
    let server = super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    migrations::upgrade(&server, &main).await?;
    let v3_branches: Vec<String> = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT name FROM dolt_branches WHERE LEFT(BINARY name, 27) = BINARY 'kuru_migration_v0000000003_' LIMIT 2")
            .fetch_all(main.as_ref()),
    )
    .await
    .context("usage migration fixture v3 branch lookup deadline exceeded")??;
    ensure!(
        v3_branches.len() == 1,
        "usage migration fixture needs one exact v3 migration branch"
    );
    let v3 = server.pool(&v3_branches[0]).await?;
    let v3_head = revision(&v3).await?;
    v3.close().await;
    let usage_name = usage_ledger::BRANCH;
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH(?, ?)")
            .bind(usage_name)
            .bind(&v3_head)
            .fetch_all(main.as_ref()),
    )
    .await
    .context("usage migration fixture branch creation deadline exceeded")??;
    let usage = server.pool(usage_name).await?;
    let legacy_id = Uuid::new_v4().hyphenated().to_string();
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
            .bind(&legacy_id)
            .bind("pre-upgrade usage receipt")
            .execute(usage.as_ref()),
    )
    .await
    .context("usage migration fixture legacy receipt deadline exceeded")??;
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_COMMIT('-Am', 'Usage before retained receipts', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(usage.as_ref()),
    )
    .await
    .context("usage migration fixture legacy commit deadline exceeded")??;
    let prior_usage_head = revision(&usage).await?;
    assert_eq!(migrations::version(&usage).await?, 3);

    let reserved = ReservedAckDropProxy::reserve().await;
    let (hooks, control) =
        migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::BeforePublish);
    let hooks = hooks.with_route(migrations::MigrationBoundary::BeforePublish, reserved.port);
    let migrating_server = server.clone();
    let migrating_usage = usage.clone();
    let mut upgrading = AbortUpgradeOnDrop(tokio::spawn(async move {
        migrations::upgrade_usage_with_hooks(&migrating_server, &migrating_usage, &hooks).await
    }));
    let source = tokio::time::timeout(TEST_DEADLINE, control.route_source())
        .await
        .context("usage upgrade did not expose publication route")??;
    let target = control.publication_target()?;
    let attempt = control.branch_name()?;
    let observation = DurableObservation::MainAtTarget {
        target: target.clone(),
    };
    let proxy = if accepted {
        reserved.start(source, "CALL DOLT_MERGE", observation)
    } else {
        reserved.start_absent(source, observation)
    };
    control.resume_route();
    tokio::time::timeout(TEST_DEADLINE, control.reached())
        .await
        .context("usage upgrade did not reach publication boundary")??;
    assert_eq!(revision(&usage).await?, prior_usage_head);
    assert_eq!(migrations::version(&usage).await?, 3);
    control.resume();
    let upgrade_result = tokio::time::timeout(TEST_DEADLINE, &mut upgrading.0)
        .await
        .context("usage upgrade did not settle its publication attempt")??;
    if accepted {
        upgrade_result?;
    } else {
        let error = upgrade_result.expect_err("undispatched usage publication appeared committed");
        ensure!(
            format!("{error:#}").contains("Dolt migration fast-forward failed"),
            "unexpected absent usage publication result: {error:#}"
        );
    }
    ensure!(
        proxy.discarded.load(Ordering::Acquire),
        "usage fixture did not discard the selected publication packet"
    );
    await_flag(&proxy.session_ended, TEST_DEADLINE).await?;
    let observed_usage_head = revision(&usage).await?;
    assert_eq!(
        observed_usage_head,
        if accepted {
            target.as_str()
        } else {
            prior_usage_head.as_str()
        }
    );
    if accepted {
        migrations::validate_usage(&server, &usage).await?;
    } else {
        assert_eq!(migrations::version(&usage).await?, 3);
    }
    let attempt_head: String = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT hash FROM dolt_branches WHERE BINARY name = BINARY ?")
            .bind(&attempt)
            .fetch_one(main.as_ref()),
    )
    .await
    .context("usage migration fixture retained attempt lookup deadline exceeded")??;
    assert_eq!(attempt_head, target);
    usage.close().await;
    main.close().await;
    server.close().await?;
    proxy.close().await;

    let reopened = tokio::time::timeout(
        migration_observation_deadline(&options),
        crate::test_support::spawn_gated_open(options),
    )
    .await
    .context("reopened usage owner did not finish its bounded migration validation")??;
    let ledger = reopened.usage_ledger()?;
    let upgraded_usage = reopened
        .shared
        .usage_pool
        .lock()
        .expect("usage pool lock")
        .clone()
        .context("reopened usage pool missing")?;
    assert_eq!(migrations::version(&upgraded_usage).await?, 4);
    assert_eq!(revision(&upgraded_usage).await?, target);
    let legacy: (String, i32) = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_as("SELECT label, receipt_format FROM operations WHERE id = ?")
            .bind(&legacy_id)
            .fetch_one(upgraded_usage.as_ref()),
    )
    .await
    .context("reopened usage legacy receipt lookup deadline exceeded")??;
    assert_eq!(legacy, ("pre-upgrade usage receipt".to_owned(), 0));
    ledger.mark_new_session("after-usage-upgrade").await?;
    let count: i64 = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT COUNT(*) FROM operations").fetch_one(upgraded_usage.as_ref()),
    )
    .await
    .context("usage migration fixture retained receipt count deadline exceeded")??;
    assert_eq!(count, 2, "reopen erased or duplicated a usage receipt");
    let upgrades: i64 = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 4%'",
        )
        .fetch_one(upgraded_usage.as_ref()),
    )
    .await
    .context("usage migration fixture upgrade log deadline exceeded")??;
    assert_eq!(upgrades, 1, "reopen rebuilt the same usage migration");
    drop(ledger);
    drop(upgraded_usage);
    reopened.close().await
}

#[tokio::test]
async fn production_upgrade_reconciles_lost_fast_forward_reply_after_target_publication()
-> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", "c".repeat(64)),
    )?;
    super::tests::released_v1(&options).await?;
    let server = super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
        .bind(b"fast-forward-reply-source".as_slice())
        .bind("\"retained\"")
        .execute(main.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'fast forward reply source', '--author', ?)")
        .bind(AUTHOR)
        .fetch_all(main.as_ref())
        .await?;
    main.close().await;
    server.close().await?;

    let reserved = ReservedAckDropProxy::reserve().await;
    let (hooks, control) =
        migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::BeforePublish);
    let hooks = hooks.with_route(migrations::MigrationBoundary::BeforePublish, reserved.port);
    options.migration_hooks = Some(Arc::new(hooks));
    let opening = tokio::spawn(crate::test_support::spawn_gated_open(options.clone()));
    let source = tokio::time::timeout(TEST_DEADLINE, control.route_source())
        .await
        .context("production migration did not expose publish-route source")??;
    let target = control.publication_target()?;
    let branch = control.branch_name()?;
    let proxy = reserved.start(
        source,
        "CALL DOLT_MERGE",
        DurableObservation::MainAtTarget {
            target: target.clone(),
        },
    );
    control.resume_route();
    tokio::time::timeout(TEST_DEADLINE, control.reached())
        .await
        .context("production migration did not reach publish boundary")??;
    control.resume();

    let store = tokio::time::timeout(TEST_DEADLINE, opening)
        .await
        .context("production migration did not reconcile lost fast-forward reply")???;
    assert!(proxy.discarded.load(Ordering::Acquire));
    await_flag(&proxy.session_ended, TEST_DEADLINE).await?;
    let completed = store.revision().await?;
    let published_ancestor: String = sqlx::query_scalar("SELECT DOLT_MERGE_BASE(?, ?)")
        .bind(&target)
        .bind(&completed)
        .fetch_one(store.pool.as_ref())
        .await?;
    assert_eq!(
        published_ancestor, target,
        "current schema must descend from the reconciled v2 target"
    );
    assert_eq!(
        store.get("fast-forward-reply-source").await?,
        Some(json!("retained"))
    );
    assert_eq!(
        migrations::version(&store.pool).await?,
        migrations::CURRENT_VERSION
    );
    assert_clean_status(store.pool.as_ref()).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kuru_migrations")
            .fetch_one(store.pool.as_ref())
            .await?,
        i64::from(migrations::CURRENT_VERSION - 1)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'"
        )
        .fetch_one(store.pool.as_ref())
        .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_branches WHERE name = ?")
            .bind(&branch)
            .fetch_one(store.pool.as_ref())
            .await?,
        1
    );
    store.close().await?;
    proxy.close().await;
    let reopened = tokio::time::timeout(
        TEST_DEADLINE,
        crate::test_support::spawn_gated_open(options),
    )
    .await??;
    assert_eq!(reopened.revision().await?, completed);
    assert_eq!(
        reopened.get("fast-forward-reply-source").await?,
        Some(json!("retained"))
    );
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn absent_fast_forward_keeps_the_same_ready_attempt_for_next_open() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::open_options(
        root.path().to_owned(),
        format!("project/{}", "d".repeat(64)),
    )?;
    super::tests::released_v1(&options).await?;
    let server = super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
        .bind(b"absent-fast-forward-source".as_slice())
        .bind("\"retained\"")
        .execute(main.as_ref())
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', 'absent fast forward source', '--author', ?)")
        .bind(AUTHOR)
        .fetch_all(main.as_ref())
        .await?;
    let base = revision(&main).await?;
    main.close().await;
    server.close().await?;

    let reserved = ReservedAckDropProxy::reserve().await;
    let (hooks, control) =
        migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::BeforePublish);
    let hooks = hooks.with_route(migrations::MigrationBoundary::BeforePublish, reserved.port);
    options.migration_hooks = Some(Arc::new(hooks));
    let opening = tokio::spawn(crate::test_support::spawn_gated_open(options.clone()));
    let source = tokio::time::timeout(TEST_DEADLINE, control.route_source())
        .await
        .context("production migration did not expose absent-publish route source")??;
    let target = control.publication_target()?;
    let branch = control.branch_name()?;
    let proxy = reserved.start_absent(
        source,
        DurableObservation::MainAtTarget {
            target: target.clone(),
        },
    );
    control.resume_route();
    tokio::time::timeout(TEST_DEADLINE, control.reached())
        .await
        .context("production migration did not reach absent publish boundary")??;
    control.resume();
    let error = tokio::time::timeout(TEST_DEADLINE, opening)
        .await
        .context("absent fast-forward did not resolve")?
        .expect("missing fast-forward must not be treated as published")
        .expect_err("missing fast-forward must retain a ready attempt");
    assert!(
        proxy.discarded.load(Ordering::Acquire),
        "fixture must drop the merge request before dispatch: {error:#}"
    );
    await_flag(&proxy.session_ended, TEST_DEADLINE).await?;
    proxy.close().await;
    options.migration_hooks = None;

    let inspection = super::tests::released_server(&options).await?;
    let inspection_main = inspection.pool("main").await?;
    assert_eq!(revision(&inspection_main).await?, base);
    assert_eq!(migrations::version(&inspection_main).await?, 1);
    assert_clean_status(inspection_main.as_ref()).await;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT hash FROM dolt_branches WHERE name = ?")
            .bind(&branch)
            .fetch_one(inspection_main.as_ref())
            .await?,
        target,
        "the exact ready branch must remain for the next open"
    );
    inspection_main.close().await;
    inspection.close().await?;
    let reopened = tokio::time::timeout(
        TEST_DEADLINE,
        crate::test_support::spawn_gated_open(options),
    )
    .await??;
    let completed = reopened.revision().await?;
    let published_ancestor: String = sqlx::query_scalar("SELECT DOLT_MERGE_BASE(?, ?)")
        .bind(&target)
        .bind(&completed)
        .fetch_one(reopened.pool.as_ref())
        .await?;
    assert_eq!(
        published_ancestor, target,
        "current schema must descend from the retained ready v2 target"
    );
    assert_eq!(
        reopened.get("absent-fast-forward-source").await?,
        Some(json!("retained"))
    );
    assert_eq!(
        migrations::version(&reopened.pool).await?,
        migrations::CURRENT_VERSION
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kuru_migrations")
            .fetch_one(reopened.pool.as_ref())
            .await?,
        i64::from(migrations::CURRENT_VERSION - 1)
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT hash FROM dolt_branches WHERE name = ?")
            .bind(&branch)
            .fetch_one(reopened.pool.as_ref())
            .await?,
        target,
        "reopen must publish the pre-existing ready target without rebuilding it"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'"
        )
        .fetch_one(reopened.pool.as_ref())
        .await?,
        1,
        "reopen must not add a second migration commit"
    );
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn isolated_schema_retry_keeps_main_clean_and_reconciles_lost_fast_forward_reply() {
    let store = stopped_released_v1_store().await;
    store
        .append(SCHEMA_BOUNDARY_NAMESPACE, "user", "main history retained")
        .await
        .unwrap();
    let base = store.revision().await.unwrap();
    let source_history = history_at_head(store.pool.as_ref(), SCHEMA_BOUNDARY_NAMESPACE).await;
    let candidate = preserved_candidate(&store).await;

    let failed = exact_base_schema_branch(&store).await;
    assert_eq!(failed.base, base);
    let failed_receipt = Uuid::new_v4().to_string();
    let (mut failed_connection, failed_id) = owned_connection(&failed.view.pool).await.unwrap();
    staged_schema_transaction(&mut failed_connection, &failed_receipt, false)
        .await
        .unwrap();
    drop(failed_connection);
    await_session_end(&store.pool, failed_id, TEST_DEADLINE)
        .await
        .unwrap();

    assert_eq!(store.revision().await.unwrap(), base);
    assert_clean_status(store.pool.as_ref()).await;
    assert!(
        matches!(probe_table_at_head(store.pool.as_ref()).await, ProbeAtHead::Error(error) if error.contains(SCHEMA_BOUNDARY_TABLE)),
        "main must not expose a failed branch table at HEAD"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
            .fetch_one(store.pool.as_ref())
            .await
            .unwrap(),
        1
    );
    assert_dirty_schema_branch(&failed, &failed_receipt, &source_history).await;
    assert_candidate_unchanged(&candidate).await;

    let fresh = exact_base_schema_branch(&store).await;
    assert_eq!(fresh.base, base);
    let receipt = Uuid::new_v4().to_string();
    let (mut fresh_connection, fresh_id) = owned_connection(&fresh.view.pool).await.unwrap();
    staged_schema_transaction(&mut fresh_connection, &receipt, true)
        .await
        .unwrap();
    drop(fresh_connection);
    await_session_end(&store.pool, fresh_id, TEST_DEADLINE)
        .await
        .unwrap();
    assert_schema_commit(&fresh.view, &base, &source_history, &receipt).await;
    let target = fresh.view.revision().await.unwrap();
    assert_eq!(store.revision().await.unwrap(), base);
    assert_clean_status(store.pool.as_ref()).await;
    assert!(
        matches!(probe_table_at_head(store.pool.as_ref()).await, ProbeAtHead::Error(error) if error.contains(SCHEMA_BOUNDARY_TABLE)),
        "main must remain at the exact base until fast-forward publication"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
            .fetch_one(store.pool.as_ref())
            .await
            .unwrap(),
        1,
        "main must retain schema version 1 until publication"
    );

    let proxy = AckDropProxy::start(
        store.pool.clone(),
        "CALL DOLT_MERGE",
        DurableObservation::MainAtTarget {
            target: target.clone(),
        },
    )
    .await;
    let affected = proxy.view(&store).await;
    let (mut merge_connection, merge_id) = owned_connection(&affected.pool).await.unwrap();
    *affected.shared.uncertain.lock().unwrap() = Some(Pending {
        pool: affected.pool.clone(),
        connection: merge_id,
        receipt: Receipt::Promotion {
            base: base.clone(),
            target: target.clone(),
        },
    });
    let merge_error = sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
        .bind(&fresh.view.branch)
        .fetch_all(&mut merge_connection)
        .await
        .unwrap_err();
    assert!(
        proxy.discarded.load(Ordering::Acquire),
        "fixture must discard an actual durable DOLT_MERGE fast-forward reply: {merge_error:#}"
    );
    drop(merge_connection);
    await_session_end(&store.pool, merge_id, TEST_DEADLINE)
        .await
        .unwrap();
    assert_eq!(affected.resolve_uncertain().await.unwrap(), Some(true));
    assert!(affected.shared.uncertain.lock().unwrap().is_none());
    assert_eq!(store.revision().await.unwrap(), target);
    assert_eq!(affected.resolve_uncertain().await.unwrap(), None);

    // The original v1 main pool remains attached to its original schema. The
    // proxy owns another Arc to it, so stop and drop that fixture first; then
    // release the last main view before acquiring a fresh current-schema pool.
    let shared = store.shared.clone();
    affected.pool.close().await;
    proxy.close().await;
    drop(affected);
    drop(store);
    let current = MemoryStore {
        pool: shared.server.pool("main").await.unwrap(),
        shared,
        branch: "main".into(),
        logical_receipt: None,
    };
    assert_schema_commit(&current, &base, &source_history, &receipt).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_log WHERE commit_hash = ?")
            .bind(&target)
            .fetch_one(current.pool.as_ref())
            .await
            .unwrap(),
        1,
        "main must reconcile to the target only once"
    );
    assert_dirty_schema_branch(&failed, &failed_receipt, &source_history).await;
    assert_candidate_unchanged(&candidate).await;

    current.close().await.unwrap();
}

/// Wait for a fixture-owned flag to become true, bounded by `duration`.
///
/// `session_ended` is set from the `AckDropProxy`'s own spawned task, a task
/// distinct from (and unordered with respect to) the task driving
/// `MemoryStore::open()` that the caller is typically awaiting alongside it.
/// The flag is set only after the proxy observes the routed SQL session's
/// actual server-side teardown (`await_session_end`), which can lag the
/// socket shutdown that unblocks `open()` by an unbounded amount of
/// wall-clock time — so a bare `.load()` races. Poll instead of asserting
/// synchronously, mirroring the bounded `durable_observation` poll used
/// elsewhere in this file for the same out-of-band-condition shape.
async fn await_flag(flag: &AtomicBool, duration: Duration) -> Result<()> {
    tokio::time::timeout(duration, async {
        while !flag.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("fixture flag did not become true")
}

struct AckDropProxy {
    port: u16,
    discarded: Arc<AtomicBool>,
    session_ended: Arc<AtomicBool>,
    task: JoinHandle<Result<()>>,
}

struct ReservedAckDropProxy {
    listener: TcpListener,
    port: u16,
    discarded: Arc<AtomicBool>,
}

#[derive(Clone)]
enum DurableObservation {
    RevisionAdvanced { base: String },
    BranchAtBase { branch: String, base: String },
    MainAtTarget { target: String },
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DropKind {
    ReplyAfterDurability,
    RequestBeforeDispatch,
}

async fn durable_observation(
    observer: &MySqlPool,
    observation: &DurableObservation,
) -> Result<bool> {
    match observation {
        DurableObservation::RevisionAdvanced { base } => Ok(revision(observer).await? != *base),
        DurableObservation::BranchAtBase { branch, base } => {
            let observed: Option<String> =
                sqlx::query_scalar("SELECT hash FROM dolt_branches WHERE name = ?")
                    .bind(branch)
                    .fetch_optional(observer)
                    .await?;
            Ok(observed.as_deref() == Some(base))
        }
        DurableObservation::MainAtTarget { target } => Ok(revision(observer).await? == *target),
    }
}

impl AckDropProxy {
    async fn start(
        observer: Arc<MySqlPool>,
        statement: &'static str,
        observation: DurableObservation,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        Self::from_listener(
            listener,
            port,
            observer,
            statement,
            observation,
            Arc::new(AtomicBool::new(false)),
            DropKind::ReplyAfterDurability,
        )
    }

    fn from_listener(
        listener: TcpListener,
        port: u16,
        observer: Arc<MySqlPool>,
        statement: &'static str,
        observation: DurableObservation,
        discarded: Arc<AtomicBool>,
        drop_kind: DropKind,
    ) -> Self {
        let upstream = observer.connect_options();
        let upstream = (upstream.get_host().to_owned(), upstream.get_port());
        let session_ended = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(Fault {
            observer,
            statement,
            observation,
            discarded: discarded.clone(),
            session_ended: session_ended.clone(),
            drop_kind,
        });
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    incoming = listener.accept() => {
                        let (client, _) = incoming?;
                        let server = TcpStream::connect((upstream.0.as_str(), upstream.1)).await?;
                        let fault = fault.clone();
                        connections.spawn(async move { proxy_connection(client, server, fault).await });
                    }
                    Some(finished) = connections.join_next(), if !connections.is_empty() => { finished??; }
                }
            }
        });
        Self {
            port,
            discarded,
            session_ended,
            task,
        }
    }

    async fn view(&self, store: &MemoryStore) -> MemoryStore {
        let options = store
            .pool
            .connect_options()
            .as_ref()
            .clone()
            .host("127.0.0.1")
            .port(self.port);
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(3))
            .connect_with(options)
            .await
            .unwrap();
        MemoryStore {
            shared: store.shared.clone(),
            pool: Arc::new(pool),
            branch: store.branch.clone(),
            logical_receipt: None,
        }
    }

    async fn close(self) {
        self.task.abort();
        let result = self.task.await;
        assert!(
            result.as_ref().is_ok_and(|value| value.is_ok())
                || result.as_ref().is_err_and(|error| error.is_cancelled()),
            "packet proxy failed before expected shutdown"
        );
    }
}

impl ReservedAckDropProxy {
    async fn reserve() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        Self {
            listener,
            port,
            discarded: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start(
        self,
        observer: Arc<MySqlPool>,
        statement: &'static str,
        observation: DurableObservation,
    ) -> AckDropProxy {
        AckDropProxy::from_listener(
            self.listener,
            self.port,
            observer,
            statement,
            observation,
            self.discarded,
            DropKind::ReplyAfterDurability,
        )
    }

    fn start_absent(
        self,
        observer: Arc<MySqlPool>,
        observation: DurableObservation,
    ) -> AckDropProxy {
        AckDropProxy::from_listener(
            self.listener,
            self.port,
            observer,
            "CALL DOLT_MERGE",
            observation,
            self.discarded,
            DropKind::RequestBeforeDispatch,
        )
    }
}

struct Fault {
    observer: Arc<MySqlPool>,
    statement: &'static str,
    observation: DurableObservation,
    discarded: Arc<AtomicBool>,
    session_ended: Arc<AtomicBool>,
    drop_kind: DropKind,
}

#[derive(Default)]
struct WireState {
    authenticated: bool,
    preparing: bool,
    prepared: Option<u32>,
    discard_next: bool,
    session_id: Option<u64>,
}

async fn proxy_connection(client: TcpStream, server: TcpStream, fault: Arc<Fault>) -> Result<()> {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();
    let state = Arc::new(StdMutex::new(WireState::default()));
    let requests = state.clone();
    let statement = fault.statement;
    let request_fault = fault.clone();
    let client_to_server = async move {
        while let Some(packet) = read_packet(&mut client_read).await? {
            let (discard, session_id) = {
                let mut state = requests.lock().unwrap();
                let payload = &packet[4..];
                if state.authenticated && !payload.is_empty() {
                    match payload[0] {
                        0x16 => state.preparing = payload[1..].starts_with(statement.as_bytes()),
                        0x03 => state.discard_next = payload[1..].starts_with(statement.as_bytes()),
                        0x17 if payload.len() >= 5 => {
                            let id = u32::from_le_bytes(payload[1..5].try_into().unwrap());
                            state.discard_next = state.prepared == Some(id);
                        }
                        _ => {}
                    }
                }
                let discard = request_fault.drop_kind == DropKind::RequestBeforeDispatch
                    && state.discard_next
                    && request_fault
                        .discarded
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok();
                if discard {
                    state.discard_next = false;
                }
                (discard, state.session_id)
            };
            if discard {
                server_write.shutdown().await?;
                let id =
                    session_id.context("fixture did not observe the routed MySQL session id")?;
                await_session_end(&request_fault.observer, id, TEST_DEADLINE)
                    .await
                    .context("routed SQL session remained after the absent request")?;
                request_fault.session_ended.store(true, Ordering::Release);
                return Ok::<_, anyhow::Error>(());
            }
            server_write.write_all(&packet).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let server_to_client = async move {
        while let Some(packet) = read_packet(&mut server_read).await? {
            let (discard, session_id) = {
                let mut state = state.lock().unwrap();
                let payload = &packet[4..];
                if !state.authenticated && payload.first() == Some(&0x0a) {
                    let Some(version_end) = payload[1..].iter().position(|byte| *byte == 0) else {
                        bail!("fixture received malformed MySQL handshake");
                    };
                    let id_start = version_end + 2;
                    ensure!(
                        payload.len() >= id_start + 4,
                        "fixture received truncated MySQL handshake"
                    );
                    let id =
                        u32::from_le_bytes(payload[id_start..id_start + 4].try_into().unwrap());
                    state.session_id = Some(u64::from(id));
                }
                if !state.authenticated && packet[3] >= 2 && payload.first() == Some(&0) {
                    state.authenticated = true;
                }
                if state.preparing && payload.len() >= 5 && payload[0] == 0 {
                    state.prepared = Some(u32::from_le_bytes(payload[1..5].try_into().unwrap()));
                    state.preparing = false;
                }
                let discard = fault.drop_kind == DropKind::ReplyAfterDurability
                    && std::mem::take(&mut state.discard_next)
                    && fault
                        .discarded
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok();
                (discard, state.session_id)
            };
            if discard {
                // Receiving a packet alone may only mean result metadata is
                // ready. Observe the durable branch revision directly before
                // discarding the reply, so the injected fault is a lost ack.
                tokio::time::timeout(TEST_DEADLINE, async {
                    loop {
                        if durable_observation(&fault.observer, &fault.observation).await? {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .context("intercepted SQL did not become durable")??;
                client_write.shutdown().await?;
                let id =
                    session_id.context("fixture did not observe the routed MySQL session id")?;
                await_session_end(&fault.observer, id, TEST_DEADLINE)
                    .await
                    .context("routed SQL session remained after the lost reply")?;
                fault.session_ended.store(true, Ordering::Release);
                return Ok::<_, anyhow::Error>(());
            }
            client_write.write_all(&packet).await?;
        }
        Ok(())
    };
    let ((), ()) = tokio::try_join!(client_to_server, server_to_client)?;
    Ok(())
}

async fn read_packet(reader: &mut (impl AsyncRead + Unpin)) -> Result<Option<Vec<u8>>> {
    let mut header = [0_u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    }
    let length =
        usize::from(header[0]) | usize::from(header[1]) << 8 | usize::from(header[2]) << 16;
    ensure!(length <= FRAME_LIMIT, "fixture MySQL packet exceeds bound");
    let mut packet = vec![0; 4 + length];
    packet[..4].copy_from_slice(&header);
    reader.read_exact(&mut packet[4..]).await?;
    Ok(Some(packet))
}
