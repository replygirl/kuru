//! The two standalone native image steps. A worker retains their exact stage,
//! lifecycle exclusion and child until checked cleanup, independently of its
//! awaiting caller. SQL validation remains with the ordinary SQL supervisor.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(unix)]
use anyhow::bail;
use anyhow::{Context, Result, ensure};
use kuru_platform::fs::Directory;
use tokio::sync::oneshot;

use super::{BackupCancellation, NativeOutput, SQL_TIMEOUT, drain_native_output};

#[cfg(unix)]
use kuru_platform::unix::{
    GroupPresence, OwnedProcessGroup, PreReap, Reap, RootState, StdioPlan, StdioSlot, Termination,
};
#[cfg(unix)]
type Owner = OwnedProcessGroup;
#[cfg(windows)]
type Owner = kuru_platform::windows::process::NativeChild;

const POLL: Duration = Duration::from_millis(10);
const CLEANUP: Duration = crate::server::SUPERVISOR_REAP_ALLOWANCE;

/// Named private stages are preserved on failure. This retained object owns
/// the optional restore permit; neither caller cancellation nor a bounded
/// cleanup refusal can release it while the native worker remains unsettled.
pub(super) struct ImageStage {
    pub(super) directory: Directory,
    binary: PathBuf,
    home: PathBuf,
    lifecycle_root: Option<PathBuf>,
    _maintenance: Option<crate::service::MaintenancePermit>,
    #[cfg(test)]
    launch_pause: std::sync::Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
}

impl ImageStage {
    pub(super) fn new(
        directory: Directory,
        binary: PathBuf,
        lifecycle_root: Option<PathBuf>,
        maintenance: Option<crate::service::MaintenancePermit>,
    ) -> Result<Arc<Self>> {
        directory.revalidate()?;
        let home = directory.path().join("home");
        crate::provision::prepare_private_home(&home)?;
        // Pinned native restore first initializes its absent destination, then
        // replaces that initial dataset root with the captured backup. Its
        // initializer requires an author even though no source history is
        // authored here. Supply only fixed local identity in this fresh home;
        // never consult the caller's Dolt configuration or credentials.
        let config = home.join("root/.dolt/config_global.json");
        let mut values: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&crate::files::read_bytes(&config, 64 * 1024)?)?;
        values.insert("user.name".into(), "Kuru".into());
        values.insert("user.email".into(), "kuru@localhost".into());
        crate::files::write_dolt_config(&config, &serde_json::to_vec(&values)?)?;
        crate::files::ensure_private_directory(&directory.path().join("data"))?;
        Ok(Arc::new(Self {
            directory,
            binary,
            home,
            lifecycle_root,
            _maintenance: maintenance,
            #[cfg(test)]
            launch_pause: std::sync::Mutex::new(None),
        }))
    }

    #[cfg(test)]
    pub(super) fn pause_next_launch(&self) -> Result<(oneshot::Receiver<()>, oneshot::Sender<()>)> {
        let (observed, arrival) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        let mut pause = self.launch_pause.lock().expect("native launch pause mutex");
        ensure!(pause.is_none(), "native launch pause already installed");
        *pause = Some((observed, wait));
        Ok((arrival, release))
    }

    #[cfg(test)]
    async fn hold_launched_owner(&self, cancellation: &BackupCancellation) {
        let pause = self
            .launch_pause
            .lock()
            .expect("native launch pause mutex")
            .take();
        if let Some((observed, wait)) = pause {
            let _ = observed.send(());
            tokio::select! {
                _ = wait => {},
                _ = cancellation.cancelled() => {},
            }
        }
    }

    pub(super) async fn restore(
        self: &Arc<Self>,
        image: PathBuf,
        cancellation: &BackupCancellation,
    ) -> Result<()> {
        let url = url::Url::from_file_path(image)
            .map_err(|_| anyhow::anyhow!("backup image has no native file URL"))?;
        self.run(Step::Restore(url), cancellation).await
    }

    pub(super) async fn fsck(self: &Arc<Self>, cancellation: &BackupCancellation) -> Result<()> {
        self.run(Step::Fsck, cancellation).await
    }

    /// Discard only after every native step returned its real cleanup result.
    /// The stopped-stage seal is checked against the retained directory;
    /// another path's lease never authorizes removal of this stage.
    pub(super) async fn discard(self: Arc<Self>) -> Result<()> {
        self.directory.revalidate()?;
        let seal = crate::server::Server::quiescence_at(
            self.directory.path(),
            self.lifecycle_root.as_deref(),
            CLEANUP,
        )
        .await?;
        ensure!(
            seal.directory.identity() == self.directory.identity(),
            "native validation cleanup selected a different directory"
        );
        self.directory.revalidate()?;
        drop(self);
        // The exact seal still excludes a new owner through native removal.
        seal.remove_tree()
            .context("remove settled private validation stage")?;
        Ok(())
    }

    /// Read the copied graph on an independent reactor. The copied SQL origin
    /// is verified before bootstrap; no original image or source owner is
    /// modified. Reply and synchronous Drop/join follow actual owned reap.
    pub(super) async fn inspect(
        self: &Arc<Self>,
        source: super::BackupSource,
        supervisor: PathBuf,
        cancellation: &BackupCancellation,
    ) -> Result<crate::store::backup_validation::Inspection> {
        self.sql_step(source, supervisor, cancellation, None)
            .await?
            .context("native inspection returned no image observations")
    }

    pub(super) async fn prepare_restore(
        self: &Arc<Self>,
        source: super::BackupSource,
        supervisor: PathBuf,
        cancellation: &BackupCancellation,
        preparation: crate::store::backup_restore::Preparation,
    ) -> Result<()> {
        ensure!(
            self.sql_step(source, supervisor, cancellation, Some(preparation))
                .await?
                .is_none(),
            "native restore preparation returned inspection observations"
        );
        Ok(())
    }

    /// Both concrete SQL jobs keep the same private server on an independent
    /// reactor and return only after its exact stopped-directory proof.
    async fn sql_step(
        self: &Arc<Self>,
        source: super::BackupSource,
        supervisor: PathBuf,
        cancellation: &BackupCancellation,
        preparation: Option<crate::store::backup_restore::Preparation>,
    ) -> Result<Option<crate::store::backup_validation::Inspection>> {
        let stage = self.clone();
        let requested = cancellation.clone();
        let (reply, result) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("kuru-backup-validation".into())
            .spawn(move || {
                let outcome = (|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    let mut server = None;
                    stage.directory.revalidate()?;
                    let restored_root = super::pinned_nbs_root(
                        &stage.directory.path().join("data/kuru/.dolt/noms"),
                        kuru_platform::fs::Privacy::Inherited,
                    )?;
                    let preparing = preparation.is_some();
                    let inspected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        runtime.block_on(async {
                            ensure!(
                                !requested.is_cancelled(),
                                "backup validation cancelled before start"
                            );
                            if preparing {
                                crate::server::verify_restored_identity(
                                    stage.directory.path(), &source.storage_scope,
                                    &source.store_instance, &source.sql_origin_instance,
                                    &source.sql_origin_scope,
                                )?;
                            } else {
                                crate::server::stage_restored_identity(
                                    stage.directory.path(), &source.storage_scope,
                                    &source.sql_origin_instance, &source.sql_origin_scope,
                                )?;
                            }
                            server = Some(
                                crate::server::Server::open(crate::server::ServerOptions {
                                    expected_instance: None,
                                    binary: stage.binary.clone(),
                                    directory: stage.directory.path().to_owned(),
                                    project_scope: source.storage_scope,
                                    supervisor,
                                    timeout: Duration::from_secs(30),
                                    read_only: false,
                                    retained: None,
                                    lifecycle_root: stage.lifecycle_root.clone(),
                                    ticks: None,
                                })
                                .await?,
                            );
                            let inspection = crate::pool::within(SQL_TIMEOUT, async {
                                let server = server.as_ref().expect("private stage server opened");
                                if let Some(input) = preparation {
                                    crate::store::backup_restore::prepare(server, stage.directory.path(), input).await?;
                                    Ok(None)
                                } else {
                                    crate::store::backup_validation::inspect(server, &requested).await.map(Some)
                                }
                            });
                            tokio::pin!(inspection);
                            tokio::select! {
                                biased;
                                result = &mut inspection => result.context("native image SQL validation deadline exceeded")?,
                                _ = requested.cancelled() => Err(anyhow::anyhow!("native image SQL validation cancelled; close and reap its private server before release")),
                            }
                        })
                    }))
                    .unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("native image validation worker panicked"))
                    });
                    runtime.block_on(async {
                        // close() can hand cleanup to the existing supervisor
                        // reaper on a bounded refusal. The exact stage lease,
                        // not a second close's empty owner slot, proves reap.
                        let closing = if let Some(server) = &server {
                            server.close().await
                        } else {
                            Ok(())
                        };
                        loop {
                            match crate::server::Server::quiescence_at(
                                stage.directory.path(),
                                stage.lifecycle_root.as_deref(),
                                CLEANUP,
                            )
                            .await
                            {
                                Ok(seal) => {
                                    // A replacement pathname's lease says
                                    // nothing about the retained stage child.
                                    // Keep its directory and maintenance owned
                                    // until the exact original lease proves reap.
                                    let exact = seal.directory.identity()
                                        == stage.directory.identity()
                                        && stage.directory.revalidate().is_ok();
                                    drop(seal);
                                    if exact {
                                        break;
                                    }
                                    tokio::time::sleep(POLL).await;
                                }
                                Err(_) => tokio::time::sleep(POLL).await,
                            }
                        }
                        drop(server);
                        stage.directory.revalidate()?;
                        ensure!(preparing ||
                            super::pinned_nbs_root(
                                &stage.directory.path().join("data/kuru/.dolt/noms"),
                                kuru_platform::fs::Privacy::Inherited,
                            )? == restored_root,
                            "read-only native image validation changed its captured dataset root"
                        );
                        stage.directory.revalidate()?;
                        match (inspected, closing) {
                            (Ok(value), Ok(())) => Ok(value),
                            (Err(error), Ok(())) => Err(error),
                            (Ok(_), Err(error)) => Err(error
                                .context("native image validation cleanup required retained reap")),
                            (Err(error), Err(cleanup)) => Err(error.context(format!(
                                "native image validation cleanup also refused: {cleanup:#}"
                            ))),
                        }
                    })
                })();
                drop(stage);
                let _ = reply.send(outcome);
            })
            .context("start native image validation worker")?;
        let mut worker = WorkerGuard::new(thread, cancellation.clone());
        let outcome = result
            .await
            .context("native image validation worker stopped without an outcome");
        worker.join();
        outcome?
    }

    async fn run(self: &Arc<Self>, step: Step, cancellation: &BackupCancellation) -> Result<()> {
        let lifecycle = crate::server::Server::quiescence_at(
            self.directory.path(),
            self.lifecycle_root.as_deref(),
            CLEANUP,
        )
        .await?;
        let stage = self.clone();
        let (reply, result) = oneshot::channel();
        let worker_cancelled = cancellation.clone();
        let thread = std::thread::Builder::new()
            .name("kuru-native-image".into())
            .spawn(move || {
                // These concrete resources stay in this thread across a bounded
                // refusal and continued cleanup. No task retains its own sender.
                let _lifecycle = lifecycle;
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let mut retained = Retained::default();
                let outcome = match &runtime {
                    Ok(runtime) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        runtime.block_on(run_owned(
                            &stage,
                            step,
                            &reply,
                            &worker_cancelled,
                            &mut retained,
                        ))
                    }))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("native image worker panicked"))),
                    Err(_) => Err(anyhow::anyhow!("native image worker runtime failed")),
                };
                if let Ok(runtime) = runtime {
                    runtime.block_on(async {
                        while retained.cleanup(Instant::now() + CLEANUP).await.is_err() {
                            tokio::time::sleep(POLL).await;
                        }
                    });
                }
                // The stage and maintenance permit release only after the same
                // retained child and readers have actually settled.
                drop(stage);
                let _ = reply.send(outcome);
            })
            .context("start native image worker")?;
        let mut worker = WorkerGuard::new(thread, cancellation.clone());
        let outcome = result
            .await
            .context("native image worker stopped without an outcome");
        // A completed step must not cancel subsequent validation on this same
        // operation. The reply follows retained cleanup; join its exact worker.
        worker.join();
        outcome?
    }
}

// This is a last-resort synchronous ownership handoff, not a detached
// cleanup task. Its dedicated runtime does not need the caller's executor.
// Ordinary CLI/service cancellation keeps and awaits the operation; a library
// caller that drops it still cannot finish/process-exit before native cleanup.
pub(super) struct WorkerGuard {
    thread: Option<std::thread::JoinHandle<()>>,
    cancellation: BackupCancellation,
}

impl WorkerGuard {
    pub(super) fn new(
        thread: std::thread::JoinHandle<()>,
        cancellation: BackupCancellation,
    ) -> Self {
        Self {
            thread: Some(thread),
            cancellation,
        }
    }

    pub(super) fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.cancellation.cancel();
            self.join();
        }
    }
}

enum Step {
    Restore(url::Url),
    Fsck,
}

#[derive(Default)]
struct Retained {
    owner: Option<Owner>,
    readers: Vec<tokio::task::JoinHandle<Result<NativeOutput>>>,
    #[cfg(test)]
    diagnostics: Vec<NativeOutput>,
}

impl Retained {
    async fn cleanup(&mut self, deadline: Instant) -> Result<()> {
        if let Some(owner) = &mut self.owner {
            cleanup_owner(owner, deadline).await?;
            self.owner.take();
        }
        // Dropping/aborting an asynchronous reader is not EOF evidence.
        while let Some(reader) = self.readers.last_mut() {
            let result = tokio::time::timeout_at(deadline.into(), reader)
                .await
                .context("native image output cleanup remains unsettled")?;
            self.readers.pop();
            let output = result.context("native image output reader stopped")??;
            #[cfg(test)]
            self.diagnostics.push(output);
            #[cfg(not(test))]
            let _ = output;
        }
        Ok(())
    }
}

async fn run_owned(
    stage: &ImageStage,
    step: Step,
    reply: &oneshot::Sender<Result<()>>,
    cancelled: &BackupCancellation,
    retained: &mut Retained,
) -> Result<()> {
    ensure!(
        !reply.is_closed() && !cancelled.is_cancelled(),
        "native image caller left before launch"
    );
    stage.directory.revalidate()?;
    let (cwd, arguments) = match step {
        Step::Restore(url) => (
            stage.directory.path().join("data"),
            vec![
                "backup".into(),
                "restore".into(),
                url.as_str().into(),
                "kuru".into(),
            ],
        ),
        Step::Fsck => (
            stage.directory.path().join("data/kuru"),
            vec!["fsck".into(), "--quiet".into()],
        ),
    };
    spawn(stage, cwd, arguments, retained).await?;
    #[cfg(test)]
    stage.hold_launched_owner(cancelled).await;
    let deadline = Instant::now() + SQL_TIMEOUT;
    let status = loop {
        ensure!(
            !reply.is_closed() && !cancelled.is_cancelled(),
            "native image caller left; cleanup required"
        );
        ensure!(
            Instant::now() < deadline,
            "native image operation deadline exceeded; cleanup required"
        );
        if let Some(status) = completed(
            retained
                .owner
                .as_mut()
                .context("native image owner missing")?,
        )
        .await?
        {
            retained.owner.take();
            break status;
        }
        tokio::time::sleep(POLL).await;
    };
    retained.cleanup(Instant::now() + CLEANUP).await?;
    // Tests use isolated fake-data images only. Production never returns or
    // prints these private native bodies, even on a rejected operation.
    #[cfg(test)]
    if !status.success() {
        for output in &retained.diagnostics {
            eprintln!(
                "isolated native image fixture diagnostic ({} total bytes): {}",
                output.bytes,
                String::from_utf8_lossy(&output.retained[..output.retained.len().min(4096)])
            );
        }
    }
    ensure!(
        status.success(),
        "native Dolt rejected the image operation ({status})"
    );
    stage.directory.revalidate()?;
    Ok(())
}

#[cfg(unix)]
async fn spawn(
    stage: &ImageStage,
    cwd: PathBuf,
    arguments: Vec<std::ffi::OsString>,
    retained: &mut Retained,
) -> Result<()> {
    let mut command = std::process::Command::new(&stage.binary);
    command
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", stage.home.join("home"))
        .env("DOLT_ROOT_PATH", stage.home.join("root"))
        .env("TMPDIR", stage.home.join("tmp"))
        .env("DOLT_DISABLE_EVENT_FLUSH", "1")
        .current_dir(cwd);
    #[cfg(test)]
    let _creation = crate::spawn_gate::child_creation().await;
    retained.owner = Some(OwnedProcessGroup::spawn(
        command,
        StdioPlan::new(StdioSlot::Null, StdioSlot::Pipe, StdioSlot::Pipe),
    )?);
    let owner = retained
        .owner
        .as_mut()
        .context("native image owner missing")?;
    let stdout = tokio::process::ChildStdout::from_std(owner.take_stdout()?)?;
    retained
        .readers
        .push(tokio::spawn(drain_native_output(stdout)));
    let stderr = tokio::process::ChildStderr::from_std(owner.take_stderr()?)?;
    retained
        .readers
        .push(tokio::spawn(drain_native_output(stderr)));
    Ok(())
}

#[cfg(windows)]
async fn spawn(
    stage: &ImageStage,
    cwd: PathBuf,
    arguments: Vec<std::ffi::OsString>,
    retained: &mut Retained,
) -> Result<()> {
    use kuru_platform::windows::process::{NativeSpawnSpec, Stdio};
    let mut spec = NativeSpawnSpec::new(stage.binary.clone(), cwd);
    spec.args = arguments;
    spec.environment = crate::engine::environment(&stage.home)?;
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    retained.owner = Some(spec.spawn().await?);
    let owner = retained
        .owner
        .as_mut()
        .context("native image owner missing")?;
    let stdout = owner.take_stdout().context("native image stdout missing")?;
    retained
        .readers
        .push(tokio::spawn(drain_native_output(stdout)));
    let stderr = owner.take_stderr().context("native image stderr missing")?;
    retained
        .readers
        .push(tokio::spawn(drain_native_output(stderr)));
    Ok(())
}

#[cfg(unix)]
async fn completed(owner: &mut Owner) -> Result<Option<std::process::ExitStatus>> {
    match owner.root_state() {
        RootState::Running | RootState::Interrupted => Ok(None),
        RootState::Exited => {
            cleanup_owner(owner, Instant::now() + CLEANUP).await?;
            match owner.root_state() {
                RootState::Reaped(status) => Ok(Some(status)),
                _ => bail!("native image root not reaped after cleanup"),
            }
        }
        RootState::Reaped(status) => Ok(Some(status)),
        RootState::Disarmed(_) => bail!("native image ownership became unavailable"),
    }
}

#[cfg(windows)]
async fn completed(owner: &mut Owner) -> Result<Option<std::process::ExitStatus>> {
    Ok(owner.try_wait()?)
}

#[cfg(unix)]
async fn cleanup_owner(owner: &mut Owner, deadline: Instant) -> Result<()> {
    loop {
        match owner.terminate_before_reap() {
            Termination::Signalled(_) | Termination::InvalidPhase => break,
            Termination::Interrupted => {}
            Termination::Disarmed(_) => bail!("native image cleanup ownership unavailable"),
        }
        ensure!(
            Instant::now() < deadline,
            "native image cleanup unconfirmed"
        );
        tokio::time::sleep(POLL).await;
    }
    loop {
        match owner.pre_reap_step(deadline) {
            PreReap::Ready | PreReap::Reaped | PreReap::Expired => match owner.reap_if_exited() {
                Reap::Reaped(_) => break,
                Reap::NotExited | Reap::Interrupted => {}
                _ => bail!("native image cleanup ownership unavailable"),
            },
            PreReap::Pending | PreReap::ExpiredPending | PreReap::Unobserved(_) => {}
            _ => bail!("native image cleanup ownership unavailable"),
        }
        ensure!(
            Instant::now() < deadline,
            "native image cleanup unconfirmed"
        );
        owner.wait_pre_reap(POLL, deadline).await;
    }
    let mut listing = owner.permission_listing(deadline);
    loop {
        match listing.resolve(owner.presence_after_reap()).await {
            GroupPresence::Absent | GroupPresence::Recycled => return Ok(()),
            GroupPresence::Present | GroupPresence::PermissionDenied => {}
            _ => bail!("native image cleanup observation unavailable"),
        }
        ensure!(
            Instant::now() < deadline,
            "native image cleanup unconfirmed"
        );
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(windows)]
async fn cleanup_owner(owner: &mut Owner, deadline: Instant) -> Result<()> {
    if owner.try_wait()?.is_none() {
        owner.terminate()?;
    }
    owner
        .wait(deadline.saturating_duration_since(Instant::now()))
        .await?;
    Ok(())
}
