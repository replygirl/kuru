//! A private Dolt sidecar owned by a lifetime-pipe supervisor.
//!
//! Numeric PIDs are never recovery authority. The supervisor retains its child
//! until it is reaped and holds the stable lifecycle lock through that point.

use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{engine::Child, files};
use anyhow::{Context, Result, anyhow, bail, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
#[cfg(windows)]
use kuru_platform::windows::{
    pipe,
    process::{Console, Lifetime, NativeChild as SupervisorChild, NativeSpawnSpec},
};
#[cfg(unix)]
use std::{
    os::{
        fd::{AsFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Child as SupervisorChild, Command, Stdio},
};
#[cfg(unix)]
use tokio::net::unix::pipe;
#[cfg(unix)]
type LifetimeSender = pipe::Sender;
#[cfg(windows)]
type LifetimeSender = pipe::Pipe;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::{
    MySqlPool, Row,
    mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode},
};
#[cfg(test)]
use tokio::sync::oneshot;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify, OwnedMutexGuard},
    time::{Instant, sleep, timeout, timeout_at},
};
use uuid::Uuid;

/// Ordinary per-attempt window for authenticating a pool and for its identity
/// query once memory is open. An opening server never uses less than this.
pub(crate) const ORDINARY_POOL_WINDOW: Duration = Duration::from_secs(2);
const DATA_DIRECTORY_MISMATCH: &str = "memory server data directory mismatch";
const IDENTITY_MISMATCH: &str = "memory SQL project/instance identity mismatch";
const RECORD_LIMIT: usize = 64 * 1024;
const LOG_LIMIT: usize = 32 * 1024;
const CLOSE_GRACE: Duration = Duration::from_secs(8);
const KILL_GRACE: Duration = Duration::from_secs(3);
/// Supervisor-transport allowance beyond a Dolt-side budget: added to the
/// configured startup timeout for readiness and the first authenticated
/// connection, and to the Dolt stop graces for the supervisor's exit report.
pub(crate) const SUPERVISOR_TRANSPORT_ALLOWANCE: Duration = Duration::from_secs(2);
/// Bound for the supervisor to stop Dolt gracefully, kill it, and report its
/// own exit after its lifetime closes. `finish_owner` enforces it.
pub(crate) const SUPERVISOR_REAP_ALLOWANCE: Duration = CLOSE_GRACE
    .saturating_add(KILL_GRACE)
    .saturating_add(SUPERVISOR_TRANSPORT_ALLOWANCE);
/// A dropped owner's background observer warns only once the supervisor has
/// outlived the reap allowance by this margin; it tracks that allowance
/// rather than defining a separate budget.
const DROPPED_REAP_WARNING_MARGIN: Duration = Duration::from_secs(1);

/// Worst-case owned close, as bounded by `close_pools_and_owner`: the first
/// graceful pool drain, the Windows lifetime close, the supervisor reap
/// allowance in `finish_owner`, and the post-reap pool drain.
#[cfg(test)]
pub(crate) fn close_budget() -> Duration {
    CLOSE_GRACE
        .saturating_add(KILL_GRACE)
        .saturating_add(SUPERVISOR_REAP_ALLOWANCE)
        .saturating_add(CLOSE_GRACE)
}

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub binary: PathBuf,
    pub directory: PathBuf,
    pub project_scope: String,
    pub supervisor: PathBuf,
    pub timeout: Duration,
    pub read_only: bool,
    pub retained: Option<Arc<tempfile::TempDir>>,
    /// Windows requires one stable external namespace for all names of this
    /// physical store. Unix preserves its existing in-directory lease layout.
    pub lifecycle_root: Option<PathBuf>,
}

#[derive(Clone)]
pub struct Server(Arc<ServerInner>);

struct ServerInner {
    directory: PathBuf,
    identity: Identity,
    endpoint: Endpoint,
    read_only: bool,
    pools: Mutex<BTreeMap<String, Weak<MySqlPool>>>,
    pool_admission: Mutex<BTreeMap<String, Weak<Mutex<()>>>>,
    /// The deadline that bounded this owned start's readiness and first
    /// authenticated probe. Pools the store opening requests from this server
    /// continue that one startup operation until the store reports ready.
    opening_deadline: StdMutex<Option<Instant>>,
    #[cfg(test)]
    candidate_wait_observer: Mutex<Option<oneshot::Sender<()>>>,
    #[cfg(test)]
    next_pool_probe_delay: StdMutex<Option<(Duration, Arc<AtomicBool>)>>,
    owner: Mutex<Option<Owner>>,
    reap_guard: Arc<StdMutex<Option<File>>>,
    closed: AtomicBool,
}

/// One branch's pool admission fence: while it lives, [`Server::pool`] cannot
/// open a Kuru session on the branch.
pub(crate) struct BranchAdmission {
    branch: String,
    _gate: OwnedMutexGuard<()>,
}

/// A checked Dolt branch procedure, built with the SQLx lifetime of the
/// admission it was proven under.
pub(crate) type BranchProcedure<'a> =
    sqlx::query::Query<'a, sqlx::MySql, sqlx::mysql::MySqlArguments>;

/// Proof that the server has ended every session on one fenced branch.
///
/// Only [`Server::retire_branch_sessions`] constructs it (its field is private
/// to this module), and the only rename, delete and exclusion-probe
/// procedures for a branch are built from it. A call site therefore cannot
/// rename or delete a branch without first retiring its pool and awaiting
/// server-observed session end. It borrows the admission fence, and each
/// procedure it builds carries that borrow, so the fence cannot be released
/// before the procedure has run. Not `Clone`: rename and delete consume it.
#[must_use = "a branch's sessions were retired to run a branch procedure"]
pub(crate) struct SessionsEnded<'a> {
    admission: &'a BranchAdmission,
}

impl<'a> SessionsEnded<'a> {
    /// The branch whose sessions ended.
    pub(crate) fn branch(&self) -> &'a str {
        &self.admission.branch
    }

    /// The checked rename of this branch to `to`.
    pub(crate) fn rename(self, to: &str) -> BranchProcedure<'a> {
        sqlx::query("CALL DOLT_BRANCH('-m', ?, ?)")
            .bind(self.admission.branch.as_str())
            .bind(to.to_owned())
    }

    /// The checked self-rename that asks the server itself to confirm no
    /// session holds this branch; it leaves the ref unchanged on success.
    pub(crate) fn exclusion_probe(&self) -> BranchProcedure<'a> {
        sqlx::query("CALL DOLT_BRANCH('-m', ?, ?)")
            .bind(self.admission.branch.as_str())
            .bind(self.admission.branch.as_str())
    }

    /// Delete this branch: checked (`-d`), or `-D` when `force`, which skips
    /// Dolt's in-use check and so needs a prior [`Self::exclusion_probe`].
    pub(crate) fn delete(self, force: bool) -> BranchProcedure<'a> {
        let flag = if force { "-D" } else { "-d" };
        sqlx::query("CALL DOLT_BRANCH(?, ?)")
            .bind(flag)
            .bind(self.admission.branch.as_str())
    }
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("directory", &self.0.directory)
            .field("instance", &self.0.identity.instance)
            .field("read_only", &self.0.read_only)
            .field("closed", &self.0.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

// Dropping this closes stdin. Do not kill the supervisor on drop: it needs to
// receive EOF and reap Dolt, including when an open/close future is cancelled.
struct Owner {
    child: Option<SupervisorChild>,
    lifetime: Option<LifetimeSender>,
    retained: Option<Arc<tempfile::TempDir>>,
    reap_guard: Arc<StdMutex<Option<File>>>,
    #[cfg(test)]
    reaped_observer: Option<oneshot::Sender<()>>,
    /// Test-support measurement only: the data directory this owner serves.
    #[cfg(any(test, feature = "test-support"))]
    trace_directory: PathBuf,
    /// Test-support only: this owner stays live in the fixture ledger until
    /// this process has reaped its supervisor.
    #[cfg(any(test, feature = "test-support"))]
    ledger: Option<crate::test_support::engine_ledger::LiveEngine>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        drop(self.lifetime.take());
        let Some(child) = self.child.take() else {
            return;
        };
        let retained = self.retained.take();
        let guard = self
            .reap_guard
            .lock()
            .expect("memory reap guard lock")
            .take();
        // Test-support only: the fixture ledger now waits for this reaper's
        // report instead of recording when the owner drops.
        #[cfg(any(test, feature = "test-support"))]
        let reaper = crate::test_support::engine_ledger::Reaper::handoff(self.ledger.take());
        #[cfg(any(test, feature = "test-support"))]
        crate::test_support::lifecycle_trace::event(
            "owner_dropped_live",
            format_args!(
                "retained={} directory={}",
                retained.is_some(),
                self.trace_directory.display()
            ),
        );
        // Independent of Tokio: tests and CLI shutdown may destroy the runtime
        // immediately after the last store handle. Keep fixture files until the
        // supervisor has confirmed that Dolt is reaped.
        std::thread::spawn(move || {
            let _guard = guard;
            observe_supervisor(
                child,
                retained,
                SUPERVISOR_REAP_ALLOWANCE + DROPPED_REAP_WARNING_MARGIN,
                SupervisorChild::try_wait,
            );
            // Test-support only. This thread may still run while the test
            // process exits, so it runs no ledger code: it reports the reap,
            // and the ledger records it on the next thread that reads it.
            #[cfg(any(test, feature = "test-support"))]
            reaper.report();
        });
    }
}

fn observe_supervisor(
    mut child: SupervisorChild,
    retained: Option<Arc<tempfile::TempDir>>,
    warn_after: Duration,
    mut status: impl FnMut(&mut SupervisorChild) -> std::io::Result<Option<std::process::ExitStatus>>,
) {
    let deadline = std::time::Instant::now() + warn_after;
    let mut warned = false;
    loop {
        let result = status(&mut child);
        if matches!(result, Ok(Some(_))) {
            return;
        }
        if !warned && (result.is_err() || std::time::Instant::now() >= deadline) {
            eprintln!(
                "memory supervisor cleanup is delayed; retaining its process handle and resources{}",
                retained
                    .as_ref()
                    .map_or_else(String::new, |directory| format!(
                        " at {}",
                        directory.path().display()
                    ))
            );
            warned = true;
        }
        // This is continued observation of the same retained process. A close
        // deadline or failed status query grants no permission to delete data.
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    version: u32,
    instance: String,
    project_scope: String,
    password: String,
    reader_password: String,
    initialized: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    instance: String,
    port: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    binary: PathBuf,
    directory: PathBuf,
    project_scope: String,
    timeout_millis: u64,
    read_only: bool,
    lifecycle_root: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Response {
    Ready { endpoint: Endpoint, owned: bool },
    Failed(String),
}

async fn drain_closed_pools(pools: &[Arc<MySqlPool>]) {
    // `Pool::close` marks its pool closed before returning the future. Build
    // all futures first so one slow MySQL QUIT cannot leave a sibling branch
    // pool admitting work while the exact owner is being reaped.
    let drains = pools.iter().map(|pool| pool.close()).collect::<Vec<_>>();
    futures::future::join_all(drains).await;
}

async fn close_pools_and_owner(pools: &[Arc<MySqlPool>], owner: Option<Owner>) -> Result<()> {
    // Mark every pool closed and first allow ordinary graceful SQL teardown.
    // SQLx can stall while gracefully closing an idle MySQL socket even after
    // the accepted query's server session has ended. Reaping our exact engine
    // can unblock that close; a timeout alone never authorizes releasing its
    // lifecycle lease or reporting a completed shutdown.
    let mut owner = owner;
    let first_drain = timeout(CLOSE_GRACE, drain_closed_pools(pools)).await;
    let reaped = if let Some(owned) = owner.as_mut() {
        finish_owner(owned).await?;
        true
    } else {
        false
    };
    if first_drain.is_err() {
        ensure!(
            reaped,
            "memory pool close deadline exceeded without an owned engine to reap"
        );
        timeout(CLOSE_GRACE, drain_closed_pools(pools))
            .await
            .context(
                "post-reap memory pool close deadline exceeded after initial graceful drain",
            )?;
    }
    // Retain owner bookkeeping and any startup guard until the final pool
    // drain settles. The supervisor's lifecycle lease ends at child reap.
    drop(owner);
    Ok(())
}

impl Server {
    /// Test-only observation after the retained child has actually reaped.
    /// Unlike acquiring the lifecycle lease, this can fire while a slow pool
    /// drain correctly keeps store ownership held.
    #[cfg(all(test, unix))]
    pub(crate) async fn observe_next_owner_reap(&self) -> Result<oneshot::Receiver<()>> {
        let mut owner = self.0.owner.lock().await;
        let owner = owner.as_mut().context("memory server has no owned child")?;
        ensure!(
            owner.reaped_observer.is_none(),
            "memory owner reap observer is already installed"
        );
        let (sender, receiver) = oneshot::channel();
        owner.reaped_observer = Some(sender);
        Ok(receiver)
    }

    pub(crate) fn instance(&self) -> &str {
        &self.0.identity.instance
    }

    /// Hold the same stable lease as the supervisor until the caller completes
    /// a stopped-store rename and directory fsync. Closing an attached handle is
    /// not proof of quiescence: only acquiring this lease establishes it.
    #[cfg(all(test, unix))]
    pub(crate) async fn quiescence(directory: &Path, wait: Duration) -> Result<LifecycleLease> {
        Self::quiescence_at(directory, None, wait).await
    }

    pub(crate) async fn quiescence_at(
        directory: &Path,
        lifecycle_root: Option<&Path>,
        wait: Duration,
    ) -> Result<LifecycleLease> {
        ensure!(
            !wait.is_zero() && wait <= Duration::from_secs(300),
            "invalid memory quiescence timeout"
        );
        prepare_directory(directory, true)?;
        let directory = fs::canonicalize(directory)?;
        let lease = LifecycleLease::new(&directory, lifecycle_root)?;
        let deadline = Instant::now() + wait;
        loop {
            lease.verify()?;
            match lease.lock.try_lock() {
                Ok(()) => {
                    lease.verify()?;
                    return Ok(lease);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    ensure!(
                        Instant::now() < deadline,
                        "memory lifecycle remains active; refusing to move its directory"
                    );
                    sleep(
                        Duration::from_millis(10)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    )
                    .await;
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }

    pub async fn open(options: ServerOptions) -> Result<Self> {
        Self::open_inner(options, Arc::new(StdMutex::new(None))).await
    }

    /// Start an owning sidecar while retaining a project startup lock through
    /// every await and any supervisor-reaper handoff.  Store migration and
    /// staging callers use this before the server can spawn or authenticate.
    pub(crate) async fn open_with_guard(options: ServerOptions, guard: File) -> Result<Self> {
        Self::open_inner(options, Arc::new(StdMutex::new(Some(guard)))).await
    }

    async fn open_inner(
        options: ServerOptions,
        reap_guard: Arc<StdMutex<Option<File>>>,
    ) -> Result<Self> {
        Self::open_inner_with_probe_delay(options, reap_guard, None, None).await
    }

    #[cfg(test)]
    pub(crate) async fn open_with_initial_probe_delay(
        options: ServerOptions,
        delay: Duration,
        entered: Arc<AtomicBool>,
    ) -> Result<Self> {
        let spawn_guard = crate::spawn_gate::spawning().await;
        Self::open_inner_with_probe_delay(
            options,
            Arc::new(StdMutex::new(None)),
            Some((delay, entered)),
            Some(spawn_guard),
        )
        .await
    }

    async fn open_inner_with_probe_delay(
        options: ServerOptions,
        reap_guard: Arc<StdMutex<Option<File>>>,
        _initial_probe_delay: Option<(Duration, Arc<AtomicBool>)>,
        _test_spawn_guard: Option<tokio::sync::RwLockReadGuard<'static, ()>>,
    ) -> Result<Self> {
        ensure!(
            options.timeout >= Duration::from_millis(1)
                && options.timeout <= Duration::from_secs(300),
            "invalid memory server startup timeout"
        );
        ensure!(
            !options.project_scope.is_empty() && options.project_scope.len() <= 4096,
            "invalid memory project scope"
        );
        prepare_directory(&options.directory, options.read_only)
            .context("prepare private memory directory before startup")?;
        let directory = fs::canonicalize(&options.directory)
            .context("resolve private memory directory before startup")?;
        LifecycleLease::validate_root(&directory, options.lifecycle_root.as_deref())
            .context("validate memory lifecycle directory before startup")?;
        if let Some(identity) = load_identity(&directory, &options.project_scope)
            .context("read memory identity before supervisor startup")?
        {
            // Validate a published endpoint even for a writer, but only readers
            // may borrow another parent's lifetime. A writer must wait for the
            // stable lease and retain its own supervisor before exposing pools.
            if let Some(endpoint) = live_endpoint(&directory, &identity, options.read_only)
                .await
                .context("inspect existing memory endpoint before startup")?
                && options.read_only
            {
                return Ok(Self::new(
                    directory,
                    identity,
                    endpoint,
                    options.read_only,
                    None,
                    reap_guard,
                    None,
                ));
            }
        } else {
            ensure!(!options.read_only, "memory has not been initialized");
        }

        let request = Request {
            binary: options.binary.clone(),
            directory: directory.clone(),
            project_scope: options.project_scope.clone(),
            timeout_millis: options.timeout.as_millis().try_into()?,
            read_only: options.read_only,
            lifecycle_root: options.lifecycle_root.clone(),
        };
        // Readiness, the first authenticated connection, and its identity
        // check are one startup operation. The two seconds are the existing
        // supervisor-transport allowance, not a fresh budget after Ready.
        // Pools the store opening requests from this server before it reports
        // ready continue the same operation; see `Server::pool_attempt`.
        let startup_deadline = Instant::now() + options.timeout + SUPERVISOR_TRANSPORT_ALLOWANCE;
        #[cfg(unix)]
        let (mut owner, response) = {
            let mut command = Command::new(options.supervisor);
            command
                .arg("--internal-dolt-supervisor")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .current_dir(&directory)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0);
            // The supervisor is this Rust program, so instrumented test binaries
            // must retain their designated profile output through environment
            // isolation. Never pass this or parent credentials to the Dolt engine.
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                command.env("LLVM_PROFILE_FILE", profile);
            }
            // Test-support measurement only: forward the inert-by-default trace.
            #[cfg(any(test, feature = "test-support"))]
            for (name, value) in crate::test_support::lifecycle_trace::forwarded() {
                command.env(name, value);
            }
            let child = command
                .spawn()
                .context("start memory lifetime supervisor")?;
            // The test gate guards this process's child creation, not the
            // supervisor's later startup or the delayed authentication probe.
            drop(_test_spawn_guard);
            let mut owner = Owner {
                child: Some(child),
                lifetime: None,
                retained: options.retained.clone(),
                reap_guard: reap_guard.clone(),
                #[cfg(test)]
                reaped_observer: None,
                #[cfg(any(test, feature = "test-support"))]
                trace_directory: directory.clone(),
                #[cfg(any(test, feature = "test-support"))]
                ledger: Some(crate::test_support::engine_ledger::register(&directory)),
            };
            let child = owner
                .child
                .as_mut()
                .expect("newly spawned memory supervisor");
            let lifetime = child
                .stdin
                .take()
                .context("supervisor lifetime pipe missing")?;
            let output = child
                .stdout
                .take()
                .context("supervisor readiness pipe missing")?;
            owner.lifetime = Some(pipe::Sender::from_owned_fd(OwnedFd::from(lifetime))?);
            let mut output = pipe::Receiver::from_owned_fd(OwnedFd::from(output))?;
            let response = timeout_at(startup_deadline, async {
                write_frame(owner.lifetime.as_mut().expect("owned lifetime"), &request)
                    .await
                    .context("send memory supervisor startup request")?;
                read_frame::<_, Response>(&mut output)
                    .await
                    .context("read memory supervisor readiness response")
            })
            .await
            .context("memory supervisor readiness deadline exceeded")
            .and_then(|response| response);
            (owner, response)
        };
        #[cfg(windows)]
        let (mut owner, response) = {
            let listener = pipe::PrivateListener::bind()?;
            let mut command = NativeSpawnSpec::new(options.supervisor.clone(), directory.clone());
            command.args = vec![
                "--internal-dolt-supervisor".into(),
                listener.address().to_owned(),
            ];
            command.lifetime = Lifetime::TrustedSupervisor;
            command.console = Console::PrivateHidden;
            let system = kuru_platform::windows::process::system_directory()?;
            let windows = system
                .parent()
                .context("Windows system directory has no parent")?;
            command.environment = vec![
                ("SystemRoot".into(), windows.as_os_str().into()),
                ("WINDIR".into(), windows.as_os_str().into()),
                ("PATH".into(), system.into_os_string()),
            ];
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                command
                    .environment
                    .push(("LLVM_PROFILE_FILE".into(), profile));
            }
            // Test-support measurement only: forward the inert-by-default trace.
            #[cfg(any(test, feature = "test-support"))]
            command
                .environment
                .extend(crate::test_support::lifecycle_trace::forwarded());
            let child = command
                .spawn()
                .await
                .context("start memory lifetime supervisor")?;
            drop(_test_spawn_guard);
            let accept = listener.accept(
                &child,
                startup_deadline.saturating_duration_since(Instant::now()),
            );
            let mut owner = Owner {
                child: Some(child),
                lifetime: None,
                retained: options.retained.clone(),
                reap_guard: reap_guard.clone(),
                #[cfg(test)]
                reaped_observer: None,
                #[cfg(any(test, feature = "test-support"))]
                trace_directory: directory.clone(),
                #[cfg(any(test, feature = "test-support"))]
                ledger: Some(crate::test_support::engine_ledger::register(&directory)),
            };
            let response = timeout_at(startup_deadline, async {
                owner.lifetime = Some(
                    accept
                        .await
                        .context("accept memory supervisor private channel")?,
                );
                let channel = owner.lifetime.as_mut().expect("owned lifetime");
                write_frame(channel, &request)
                    .await
                    .context("send memory supervisor startup request")?;
                read_frame::<_, Response>(channel)
                    .await
                    .context("read memory supervisor readiness response")
            })
            .await
            .context("memory supervisor readiness deadline exceeded")
            .and_then(|response| response);
            (owner, response)
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return Err(startup_failure(&mut owner, error).await);
            }
        };
        let (endpoint, owned) = match response {
            Response::Ready { endpoint, owned } => {
                if !owned && !options.read_only {
                    return Err(startup_failure(
                        &mut owner,
                        anyhow!("writable memory requires an owned supervisor lifetime"),
                    )
                    .await);
                }
                (endpoint, owned)
            }
            Response::Failed(message) => {
                return Err(startup_failure(
                    &mut owner,
                    anyhow!("memory server startup failed: {message}"),
                )
                .await);
            }
        };
        let verified = async {
            let identity = load_identity(&directory, &options.project_scope)
                .context("read memory identity after supervisor readiness")?
                .context("memory identity missing after startup")?;
            let remaining = startup_deadline.saturating_duration_since(Instant::now());
            ensure!(
                !remaining.is_zero(),
                "post-readiness memory authentication deadline exceeded"
            );
            let probe = timeout_at(
                startup_deadline,
                connect_pool_with_timeout(
                    &identity,
                    &endpoint,
                    &directory,
                    "main",
                    options.read_only,
                    1,
                    PoolAttemptOptions {
                        // One transient attempt, closed after verification.
                        acquire_timeout: remaining,
                        first_acquire_window: remaining,
                        identity_rejection_is_terminal: true,
                        _test_probe_delay: _initial_probe_delay,
                    },
                ),
            )
            .await
            .context("post-readiness memory authentication deadline exceeded")?
            .context("authenticate post-readiness memory connection")?;
            let verification =
                verify_identity_until(&probe, &directory, &identity, startup_deadline)
                    .await
                    .context("verify post-readiness memory identity");
            let closed = timeout(CLOSE_GRACE, probe.close())
                .await
                .context("post-readiness memory pool close deadline exceeded");
            match (verification, closed) {
                (Ok(()), Ok(())) => Ok(identity),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(error)) => Err(error),
                (Err(error), Err(close)) => Err(error.context(format!(
                    "post-readiness memory cleanup also failed: {close:#}"
                ))),
            }
        }
        .await;
        let identity = match verified {
            Ok(identity) => identity,
            Err(error) => {
                return Err(startup_failure(&mut owner, error).await);
            }
        };
        let owner = if owned {
            Some(owner)
        } else {
            finish_owner(&mut owner).await?;
            None
        };
        Ok(Self::new(
            directory,
            identity,
            endpoint,
            options.read_only,
            owner,
            reap_guard,
            Some(startup_deadline),
        ))
    }

    fn new(
        directory: PathBuf,
        identity: Identity,
        endpoint: Endpoint,
        read_only: bool,
        owner: Option<Owner>,
        reap_guard: Arc<StdMutex<Option<File>>>,
        opening_deadline: Option<Instant>,
    ) -> Self {
        Self(Arc::new(ServerInner {
            directory,
            identity,
            endpoint,
            read_only,
            pools: Mutex::new(BTreeMap::new()),
            pool_admission: Mutex::new(BTreeMap::new()),
            opening_deadline: StdMutex::new(opening_deadline),
            #[cfg(test)]
            candidate_wait_observer: Mutex::new(None),
            #[cfg(test)]
            next_pool_probe_delay: StdMutex::new(None),
            owner: Mutex::new(owner),
            reap_guard,
            closed: AtomicBool::new(false),
        }))
    }

    pub async fn pool(&self, branch: &str) -> Result<Arc<MySqlPool>> {
        let _admission = self.fence_pool(branch).await?;
        let mut pools = self.0.pools.lock().await;
        ensure!(
            !self.0.closed.load(Ordering::Acquire),
            "memory server is closed"
        );
        if let Some(pool) = pools.get(branch).and_then(Weak::upgrade) {
            return Ok(pool);
        }
        pools.retain(|_, pool| pool.strong_count() != 0);
        let pool = connect_pool_with_timeout(
            &self.0.identity,
            &self.0.endpoint,
            &self.0.directory,
            branch,
            self.0.read_only,
            4,
            self.pool_attempt(),
        )
        .await
        .context("authenticate memory branch pool")?;
        verify_identity_until(
            &pool,
            &self.0.directory,
            &self.0.identity,
            Instant::now() + self.pool_attempt_window(),
        )
        .await
        .context("verify memory branch pool identity")?;
        let pool = Arc::new(pool);
        pools.insert(branch.to_owned(), Arc::downgrade(&pool));
        Ok(pool)
    }

    /// End the opening phase once the store that started this server is
    /// ready. Later pools use exactly the ordinary per-attempt window.
    pub(crate) fn finish_opening(&self) {
        *self
            .0
            .opening_deadline
            .lock()
            .expect("opening deadline lock") = None;
    }

    fn opening_deadline(&self) -> Option<Instant> {
        *self
            .0
            .opening_deadline
            .lock()
            .expect("opening deadline lock")
    }

    /// While opening, the remaining startup deadline that bounded this
    /// server's own probe, never less than the ordinary window: a slow but
    /// healthy start must not leave the next open-sequence pool with less
    /// than an ordinary attempt. Once open, exactly the ordinary window.
    fn pool_attempt_window(&self) -> Duration {
        self.opening_deadline()
            .map_or(ORDINARY_POOL_WINDOW, |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(ORDINARY_POOL_WINDOW)
            })
    }

    fn pool_attempt(&self) -> PoolAttemptOptions {
        let opening = self.opening_deadline().is_some();
        PoolAttemptOptions {
            // SQLx keeps this for every later acquire on the pool, so a pool
            // created while opening must still be ordinary once memory is open.
            acquire_timeout: ORDINARY_POOL_WINDOW,
            first_acquire_window: self.pool_attempt_window(),
            // A longer opening window must not wait out an identity
            // rejection; the same endpoint cannot answer differently.
            identity_rejection_is_terminal: opening,
            #[cfg(test)]
            _test_probe_delay: self
                .0
                .next_pool_probe_delay
                .lock()
                .expect("pool probe delay lock")
                .take(),
            #[cfg(not(test))]
            _test_probe_delay: None,
        }
    }

    /// Delay the next pool's authentication callback after a real Dolt
    /// connection, as `open_with_initial_probe_delay` does for the probe.
    #[cfg(test)]
    pub(crate) fn delay_next_pool_authentication(&self, delay: Duration, entered: Arc<AtomicBool>) {
        *self
            .0
            .next_pool_probe_delay
            .lock()
            .expect("pool probe delay lock") = Some((delay, entered));
    }

    /// Prevent a new pool for one branch while its checked status transition
    /// retires server sessions and observes the resulting ref. The short map
    /// lookup never holds a global lock across Dolt work.
    pub(crate) async fn fence_pool(&self, branch: &str) -> Result<BranchAdmission> {
        validate_branch(branch)?;
        let gate = {
            let mut gates = self.0.pool_admission.lock().await;
            if let Some(gate) = gates.get(branch).and_then(Weak::upgrade) {
                gate
            } else {
                gates.retain(|_, gate| gate.strong_count() != 0);
                let gate = Arc::new(Mutex::new(()));
                gates.insert(branch.to_owned(), Arc::downgrade(&gate));
                gate
            }
        };
        Ok(BranchAdmission {
            branch: branch.to_owned(),
            _gate: gate.lock_owned().await,
        })
    }

    #[cfg(test)]
    pub(crate) async fn observe_next_candidate_wait(&self) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        let mut observer = self.0.candidate_wait_observer.lock().await;
        assert!(
            observer.replace(sender).is_none(),
            "candidate wait observer already installed"
        );
        receiver
    }

    #[cfg(test)]
    pub(crate) async fn notify_candidate_wait(&self) {
        if let Some(observer) = self.0.candidate_wait_observer.lock().await.take() {
            let _ = observer.send(());
        }
    }

    /// Close Kuru's pool for `branch` without waiting for the server to end
    /// its sessions. It releases the pool early and nothing more: a branch
    /// rename or delete still requires [`Self::retire_branch_sessions`], whose
    /// [`SessionsEnded`] the branch procedures take.
    pub(crate) async fn close_pool_without_session_end(&self, branch: &str) -> Result<()> {
        self.retire_pool(branch).await
    }

    /// Retire Kuru's pool for the fenced branch, then wait until the server
    /// itself no longer lists a session on it. Closing the client pool is not
    /// enough: Dolt removes a session only when its per-connection loop
    /// observes the close, and until then a checked rename, delete or
    /// exclusion probe of the branch is refused as in use. The admission
    /// fence keeps any Kuru session from reopening the branch, and the
    /// returned proof borrows it, so the fence outlives every procedure the
    /// proof builds. `observer` must select a different branch.
    pub(crate) async fn retire_branch_sessions<'a>(
        &self,
        admission: &'a BranchAdmission,
        observer: &MySqlPool,
        deadline: Duration,
    ) -> Result<SessionsEnded<'a>> {
        self.retire_pool(&admission.branch).await?;
        self.await_branch_sessions_end(observer, &admission.branch, deadline)
            .await?;
        Ok(SessionsEnded { admission })
    }

    /// Wait, within `duration`, until the server lists no session on
    /// `kuru/<branch>`. Observation alone; it grants no branch procedure.
    pub(crate) async fn await_branch_sessions_end(
        &self,
        observer: &MySqlPool,
        branch: &str,
        duration: Duration,
    ) -> Result<()> {
        let database = format!("kuru/{branch}");
        timeout(duration, async {
            loop {
                let active: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM information_schema.processlist WHERE BINARY DB = BINARY ?",
                )
                .bind(&database)
                .fetch_one(observer)
                .await?;
                if active == 0 {
                    return Ok::<_, anyhow::Error>(());
                }
                #[cfg(test)]
                self.notify_candidate_wait().await;
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("candidate source session retirement deadline exceeded")?
    }

    /// The raw pool close. Private: outside this module a pool is released
    /// only through [`Self::close_pool_without_session_end`], whose name
    /// states what it does not guarantee, or through the ordered
    /// [`Self::retire_branch_sessions`].
    async fn retire_pool(&self, branch: &str) -> Result<()> {
        validate_branch(branch)?;
        let pool = self
            .0
            .pools
            .lock()
            .await
            .remove(branch)
            .and_then(|pool| pool.upgrade());
        if let Some(pool) = pool {
            timeout(CLOSE_GRACE, pool.close())
                .await
                .context("memory branch pool close deadline exceeded")?;
        }
        Ok(())
    }

    pub async fn close(&self) -> Result<()> {
        // The owner mutex also makes concurrent close callers wait for reaping.
        let mut owner = self.0.owner.lock().await;
        self.0.closed.store(true, Ordering::Release);
        let pools = std::mem::take(&mut *self.0.pools.lock().await)
            .values()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        close_pools_and_owner(&pools, owner.take()).await
    }

    pub(crate) fn take_reap_guard(&self) -> File {
        self.0
            .reap_guard
            .lock()
            .expect("memory reap guard lock")
            .take()
            .expect("installed memory reap guard")
    }

    pub(crate) async fn close_installed_guard(&self) -> Result<File> {
        let mut owner = self.0.owner.lock().await;
        self.0.closed.store(true, Ordering::Release);
        let pools = std::mem::take(&mut *self.0.pools.lock().await)
            .values()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        close_pools_and_owner(&pools, owner.take()).await?;
        Ok(self.take_reap_guard())
    }

    /// Transfer an ephemeral fixture's directory to the lifecycle owner. It is
    /// deleted only after the owned supervisor has reaped the database process.
    pub async fn retain_directory(
        &self,
        directory: impl Into<Arc<tempfile::TempDir>>,
    ) -> Result<()> {
        let directory = directory.into();
        let ancestor =
            Directory::open(directory.path(), Privacy::Inherited, NameRetention::Movable)?;
        ensure!(
            files::directory(&self.0.directory)?.is_within(&ancestor)?,
            "retained directory does not own memory storage"
        );
        let mut owner = self.0.owner.lock().await;
        let owner = owner
            .as_mut()
            .context("only the sidecar owner can retain a temporary directory")?;
        ensure!(
            owner.retained.is_none(),
            "temporary memory directory already retained"
        );
        owner.retained = Some(directory);
        Ok(())
    }
}

async fn finish_owner(owner: &mut Owner) -> Result<()> {
    #[cfg(windows)]
    if let Some(lifetime) = owner.lifetime.as_mut() {
        lifetime.close(KILL_GRACE).await?;
    }
    drop(owner.lifetime.take());
    let deadline = Instant::now() + SUPERVISOR_REAP_ALLOWANCE;
    let status = loop {
        if let Some(status) = owner
            .child
            .as_mut()
            .context("memory supervisor already reaped")?
            .try_wait()?
        {
            break status;
        }
        ensure!(
            Instant::now() < deadline,
            "memory supervisor shutdown deadline exceeded; lifecycle lock remains authoritative"
        );
        sleep(Duration::from_millis(20)).await;
    };
    owner.child.take();
    #[cfg(any(test, feature = "test-support"))]
    crate::test_support::lifecycle_trace::event(
        "owner_finished",
        format_args!(
            "success={} dir_exists={} directory={}",
            status.success(),
            crate::test_support::lifecycle_trace::exists(&owner.trace_directory),
            owner.trace_directory.display()
        ),
    );
    ensure!(
        status.success(),
        "memory supervisor exited unsuccessfully ({status})"
    );
    #[cfg(test)]
    if let Some(observer) = owner.reaped_observer.take() {
        let _ = observer.send(());
    }
    Ok(())
}

async fn startup_failure(owner: &mut Owner, error: anyhow::Error) -> anyhow::Error {
    match finish_owner(owner).await {
        Ok(()) => error,
        Err(cleanup) => error.context(format!("memory startup cleanup also failed: {cleanup:#}")),
    }
}

fn validate_branch(branch: &str) -> Result<()> {
    ensure!(
        !branch.is_empty()
            && branch.len() <= 128
            && branch
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'),
        "invalid memory branch"
    );
    Ok(())
}

fn private_metadata(path: &Path, directory: bool) -> Result<fs::Metadata> {
    if directory {
        files::directory(path)?;
        Ok(fs::symlink_metadata(path)?)
    } else {
        let (_directory, file) = files::read(path, Privacy::OwnerOnly)?;
        Ok(file.metadata()?)
    }
}

fn private_directory(path: &Path) -> Result<()> {
    files::private_dir(path)
}

/// The external Windows lock has no descendant file handle in the moved store.
#[derive(Debug)]
pub(crate) struct LifecycleLease {
    pub(crate) directory: Directory,
    lock_directory: Directory,
    lock_name: std::ffi::OsString,
    lock: File,
}

impl std::ops::Deref for LifecycleLease {
    type Target = File;
    fn deref(&self) -> &File {
        &self.lock
    }
}

impl LifecycleLease {
    fn validate_root(directory: &Path, root: Option<&Path>) -> Result<()> {
        #[cfg(unix)]
        {
            let _ = directory;
            ensure!(
                root.is_none(),
                "Unix memory preserves its in-directory lifecycle lock"
            );
        }
        #[cfg(windows)]
        {
            let root =
                root.context("Windows memory requires an explicit external lifecycle_root")?;
            ensure!(
                root.is_absolute(),
                "Windows lifecycle_root must be absolute"
            );
            let store = files::directory(directory)?;
            let root = files::ensure_private_directory(root)?;
            ensure!(
                !root.is_within(&store)?,
                "Windows lifecycle_root must not lie inside the moved memory store"
            );
        }
        Ok(())
    }

    fn new(directory: &Path, root: Option<&Path>) -> Result<Self> {
        Self::validate_root(directory, root)?;
        let directory = files::directory(directory)?;
        #[cfg(unix)]
        let (lock_directory, lock_name): (_, std::ffi::OsString) =
            (files::directory(directory.path())?, "lifecycle.lock".into());
        #[cfg(windows)]
        let (lock_directory, lock_name): (_, std::ffi::OsString) = {
            let root = files::open_directory(
                root.expect("validated Windows lifecycle root"),
                Privacy::OwnerOnly,
                NameRetention::Pinned,
            )?;
            let key: String = directory
                .identity()
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            (root, format!("{key}.lock").into())
        };
        let lock = lock_directory.lock_file(&lock_name)?;
        let lease = Self {
            directory,
            lock_directory,
            lock_name,
            lock,
        };
        lease.verify()?;
        Ok(lease)
    }

    fn verify(&self) -> Result<()> {
        let named = files::directory(self.directory.path())
            .context("memory lifecycle directory was moved or replaced")?;
        ensure!(
            named.identity() == self.directory.identity(),
            "memory lifecycle directory was moved or replaced"
        );
        self.lock_directory
            .verify(&self.lock_name, &self.lock)
            .context("memory lifecycle lock was moved or replaced")?;
        Ok(())
    }

    pub(crate) fn move_to(&mut self, destination: &Path) -> Result<()> {
        self.verify()?;
        #[cfg(any(test, feature = "test-support"))]
        crate::test_support::lifecycle_trace::event(
            "lease_move",
            format_args!(
                "from={} to={}",
                self.directory.path().display(),
                destination.display()
            ),
        );
        self.directory = files::move_directory(&self.directory, destination)?;
        #[cfg(unix)]
        {
            self.lock_directory = files::directory(self.directory.path())?;
        }
        self.verify()
    }

    /// Remove the stopped, still-verified store tree while retaining the
    /// lifecycle lock and its parent authority through native cleanup. This
    /// consumes the lease: an uncertain removal leaves its separately recorded
    /// quarantine identity for a later explicit reconciliation attempt.
    pub(crate) fn remove_tree(self) -> Result<()> {
        self.verify()?;
        #[cfg(any(test, feature = "test-support"))]
        crate::test_support::lifecycle_trace::event(
            "lease_remove_tree",
            format_args!("directory={}", self.directory.path().display()),
        );
        let Self {
            directory,
            lock_directory,
            lock_name,
            lock,
        } = self;
        // Keep the lifecycle authority live until the checked tree primitive
        // has consumed the root. On Windows the lifecycle root is external;
        // on Unix this is an in-tree lock whose held descriptor remains valid
        // across unlink until this scope exits.
        let _retained_lifecycle = (lock_directory, lock_name, lock);
        directory.remove_tree().map_err(anyhow::Error::from)
    }

    #[cfg(test)]
    pub(crate) fn move_to_observed(
        &mut self,
        destination: &Path,
        observer: impl FnOnce(&Directory) -> Result<()>,
    ) -> Result<()> {
        self.verify()?;
        self.directory = files::move_directory_observed(&self.directory, destination, observer)?;
        #[cfg(unix)]
        {
            self.lock_directory = files::directory(self.directory.path())?;
        }
        self.verify()
    }
}

fn prepare_directory(directory: &Path, read_only: bool) -> Result<()> {
    prepare_directory_then(directory, read_only, |_| {})
}

/// [`prepare_directory`] with a hook between listing an entry and opening it,
/// where a previous owner generation's supervisor can retire its record.
fn prepare_directory_then(
    directory: &Path,
    read_only: bool,
    mut listed: impl FnMut(&str),
) -> Result<()> {
    if read_only {
        private_metadata(directory, true).context("memory has not been initialized privately")?;
    } else {
        private_directory(directory)?;
    }
    let scanned = files::directory(directory)?;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("unrecognized memory directory entry")?;
        let is_directory = matches!(name, "data" | "config" | "home" | "staging");
        ensure!(
            is_directory
                || matches!(
                    name,
                    "identity.json"
                        | "endpoint.json"
                        | "lifecycle.lock"
                        | "server.yaml"
                        | "server.log"
                        | "migration.json"
                        | "ready.json"
                ),
            "unrecognized memory directory entry: {name}"
        );
        listed(name);
        if is_directory {
            private_metadata(&entry.path(), true)?;
            continue;
        }
        // This scan runs before its caller holds the lifecycle lease, so a
        // previous owner generation's supervisor can still be retiring its
        // endpoint record under that lease. A listed regular file whose open
        // finds its name gone, or its object unlinked, has no privacy left to
        // check. It is skipped only while the scanned directory itself is
        // unchanged; every other error, and any error for a directory entry,
        // still fails the scan.
        match private_metadata(&entry.path(), false) {
            Ok(_) => {}
            Err(error) if crate::service::is_not_found(&error) => scanned
                .revalidate()
                .context("memory directory changed while its entries were checked")?,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_record<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    read_record_then(path, || {})
}

/// [`read_record`] with a hook between its existence check and its open, where
/// a previous owner generation's supervisor can retire the record.
fn read_record_then<T: DeserializeOwned>(path: &Path, between: impl FnOnce()) -> Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => {
            result?;
        }
    }
    let parent = files::parent(path, Privacy::OwnerOnly, NameRetention::Movable)?;
    between();
    // A previous owner generation's supervisor retires its endpoint record
    // under a lease this reader does not hold, so the record can vanish after
    // the check above: at its open, or before its name is verified again after
    // the read. That NotFound is the absence the check would have reported,
    // once the parent directory is unchanged; every other error still fails.
    let bytes = match private_metadata(path, false)
        .and_then(|_| files::read_bytes(path, RECORD_LIMIT as u64))
    {
        Ok(bytes) => bytes,
        Err(error) if crate::service::is_not_found(&error) => {
            parent
                .revalidate()
                .context("memory record directory changed while the record was read")?;
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    ensure!(
        bytes.len() <= RECORD_LIMIT,
        "memory record exceeds size limit"
    );
    serde_json::from_slice(&bytes)
        .map(Some)
        .context("invalid private memory record")
}

fn write_record(path: &Path, record: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(record)?;
    ensure!(
        bytes.len() <= RECORD_LIMIT,
        "memory record exceeds size limit"
    );
    files::write(path, &bytes)
}

fn load_identity(directory: &Path, project_scope: &str) -> Result<Option<Identity>> {
    let identity: Option<Identity> = read_record(&directory.join("identity.json"))?;
    if let Some(identity) = &identity {
        ensure!(
            identity.version == 1 && identity.project_scope == project_scope,
            "memory project identity mismatch"
        );
        Uuid::parse_str(&identity.instance).context("invalid memory instance identity")?;
        for secret in [&identity.password, &identity.reader_password] {
            ensure!(
                valid_secret(secret),
                "invalid private memory credential record"
            );
        }
    }
    Ok(identity)
}

fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn valid_secret(secret: &str) -> bool {
    secret.len() == 64 && secret.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone)]
struct ConnectionObservation(Arc<StdMutex<ConnectionProgress>>);

struct ConnectionProgress {
    phase: &'static str,
    last_failure: Option<(&'static str, &'static str)>,
}

impl ConnectionObservation {
    fn new() -> Self {
        Self(Arc::new(StdMutex::new(ConnectionProgress {
            phase: "after_connect not entered",
            last_failure: None,
        })))
    }

    fn phase(&self, phase: &'static str) {
        if let Ok(mut progress) = self.0.lock() {
            progress.phase = phase;
        }
    }

    fn rejected(&self, error: &sqlx::Error) {
        // SQLx discards callback errors while retrying acquisition. Keep only
        // authored messages or static error classes, never SQL payloads or
        // connection options. A later attempt may be in a different phase.
        let Ok(mut progress) = self.0.lock() else {
            return;
        };
        let cause = match error {
            sqlx::Error::Protocol(message) if message == DATA_DIRECTORY_MISMATCH => {
                DATA_DIRECTORY_MISMATCH
            }
            sqlx::Error::Protocol(message) if message == IDENTITY_MISMATCH => IDENTITY_MISMATCH,
            sqlx::Error::Protocol(_) if progress.phase == "checked data directory comparison" => {
                "checked filesystem validation failed"
            }
            sqlx::Error::Protocol(_) => "SQL protocol failure",
            sqlx::Error::Database(_) => "database rejected verification query",
            sqlx::Error::Io(_) => "verification I/O failure",
            sqlx::Error::RowNotFound => "verification row missing",
            sqlx::Error::ColumnNotFound(_) => "verification column missing",
            _ => "SQL verification failure",
        };
        progress.last_failure = Some((progress.phase, cause));
    }

    fn diagnostic(&self) -> String {
        let Ok(progress) = self.0.lock() else {
            return "connection observation unavailable".into();
        };
        let mut diagnostic = format!("connection phase: {}", progress.phase);
        if let Some((phase, cause)) = progress.last_failure {
            diagnostic.push_str(&format!(
                "; last callback rejection during {phase}: {cause}"
            ));
        }
        diagnostic
    }

    fn is_pre_callback_connection_reset(&self, error: &sqlx::Error) -> bool {
        let Ok(progress) = self.0.lock() else {
            return false;
        };
        progress.phase == "after_connect not entered"
            && matches!(error, sqlx::Error::Io(error) if error.kind() == std::io::ErrorKind::ConnectionReset)
    }
}

struct PoolAttemptOptions {
    /// SQLx's per-acquire timeout for the whole lifetime of the pool.
    acquire_timeout: Duration,
    /// Bound on the pool's first acquisition, at least `acquire_timeout`.
    /// Acquires that time out are retried until it ends.
    first_acquire_window: Duration,
    /// SQLx retries every `after_connect` error until `acquire_timeout`.
    /// When set, an authored identity rejection ends acquisition instead.
    identity_rejection_is_terminal: bool,
    _test_probe_delay: Option<(Duration, Arc<AtomicBool>)>,
}

impl PoolAttemptOptions {
    fn ordinary() -> Self {
        Self {
            acquire_timeout: ORDINARY_POOL_WINDOW,
            first_acquire_window: ORDINARY_POOL_WINDOW,
            identity_rejection_is_terminal: false,
            _test_probe_delay: None,
        }
    }
}

/// An authored identity rejection from the authentication callback. It
/// cannot change on retry against the same endpoint.
fn identity_rejection(error: &sqlx::Error) -> Option<&'static str> {
    match error {
        sqlx::Error::Protocol(message) if message == DATA_DIRECTORY_MISMATCH => {
            Some(DATA_DIRECTORY_MISMATCH)
        }
        sqlx::Error::Protocol(message) if message == IDENTITY_MISMATCH => Some(IDENTITY_MISMATCH),
        _ => None,
    }
}

/// The first identity rejection seen by one pool attempt's callbacks.
#[derive(Clone, Default)]
struct IdentityRejection(Arc<(StdMutex<Option<&'static str>>, Notify)>);

impl IdentityRejection {
    fn record(&self, error: &sqlx::Error) {
        let Some(cause) = identity_rejection(error) else {
            return;
        };
        if let Ok(mut first) = self.0.0.lock() {
            first.get_or_insert(cause);
        }
        self.0.1.notify_one();
    }

    async fn rejected(&self) -> sqlx::Error {
        self.0.1.notified().await;
        let cause = self
            .0
            .0
            .lock()
            .ok()
            .and_then(|first| *first)
            .unwrap_or(IDENTITY_MISMATCH);
        sqlx::Error::Protocol(cause.into())
    }
}

async fn connect_pool_with_timeout(
    identity: &Identity,
    endpoint: &Endpoint,
    directory: &Path,
    branch: &str,
    read_only: bool,
    max: u32,
    attempt: PoolAttemptOptions,
) -> Result<MySqlPool> {
    let (result, observation) = connect_pool_attempt(
        identity, endpoint, directory, branch, read_only, max, attempt,
    )
    .await?;
    result.map_err(|error| connection_error(error, &observation))
}

fn connection_error(error: sqlx::Error, observation: &ConnectionObservation) -> anyhow::Error {
    anyhow::Error::from(error).context(format!(
        "connect to authenticated project memory; {}",
        observation.diagnostic()
    ))
}

async fn connect_pool_attempt(
    identity: &Identity,
    endpoint: &Endpoint,
    directory: &Path,
    branch: &str,
    read_only: bool,
    max: u32,
    attempt: PoolAttemptOptions,
) -> Result<(
    std::result::Result<MySqlPool, sqlx::Error>,
    ConnectionObservation,
)> {
    ensure!(
        endpoint.instance == identity.instance && endpoint.port >= 1024,
        "memory endpoint identity mismatch"
    );
    let options = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(endpoint.port)
        .username(if read_only { "kuru_reader" } else { "root" })
        .password(if read_only {
            &identity.reader_password
        } else {
            &identity.password
        })
        .database(&format!("kuru/{branch}"))
        .ssl_mode(MySqlSslMode::Disabled);
    let instance = identity.instance.clone();
    let project_scope = identity.project_scope.clone();
    let expected_directory = directory.join("data");
    let observation = ConnectionObservation::new();
    let callback_observation = observation.clone();
    let rejection = IdentityRejection::default();
    let callback_rejection = rejection.clone();
    // Every callback of this attempt stalls until one instant, fixed by the
    // first: a server that becomes responsive then, not a per-retry delay.
    #[cfg(test)]
    let stalled_until = Arc::new(std::sync::OnceLock::<Instant>::new());
    let pool = MySqlPoolOptions::new()
        .max_connections(max)
        .min_connections(0)
        .acquire_timeout(attempt.acquire_timeout)
        .idle_timeout(Duration::from_secs(30))
        .after_connect(move |connection, _| {
            let instance = instance.clone();
            let project_scope = project_scope.clone();
            let expected_directory = expected_directory.clone();
            let observation = callback_observation.clone();
            let rejection = callback_rejection.clone();
            #[cfg(test)]
            let test_probe_delay = attempt._test_probe_delay.clone();
            #[cfg(test)]
            let stalled_until = stalled_until.clone();
            Box::pin(async move {
                #[cfg(test)]
                if let Some((delay, entered)) = test_probe_delay {
                    observation.phase("initial authentication callback entered");
                    entered.store(true, Ordering::SeqCst);
                    tokio::time::sleep_until(*stalled_until.get_or_init(|| Instant::now() + delay))
                        .await;
                }
                let result = async {
                    observation.phase("data directory query");
                    let datadir: String = sqlx::query_scalar("SELECT @@datadir")
                        .fetch_one(&mut *connection)
                        .await?;
                    observation.phase("checked data directory comparison");
                    if !same_directory(Path::new(&datadir), &expected_directory)
                        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?
                    {
                        return Err(sqlx::Error::Protocol(DATA_DIRECTORY_MISMATCH.into()));
                    }
                    observation.phase("SQL project/instance query");
                    let row = sqlx::query(
                        "SELECT instance_id, project_scope FROM kuru_instance WHERE singleton = 1",
                    )
                    .fetch_one(connection)
                    .await?;
                    observation.phase("SQL project/instance comparison");
                    if row.try_get::<String, _>("instance_id")? != instance
                        || row.try_get::<String, _>("project_scope")? != project_scope
                    {
                        return Err(sqlx::Error::Protocol(IDENTITY_MISMATCH.into()));
                    }
                    observation.phase("authenticated identity callback complete");
                    Ok(())
                }
                .await;
                if let Err(error) = &result {
                    observation.rejected(error);
                    rejection.record(error);
                }
                result
            })
        })
        .connect_lazy_with(options);
    // Equivalent to `connect_with` (one acquire, then release) when the first
    // window equals the lifetime timeout. A longer opening window retries
    // timed-out acquires without changing any later acquire on this pool.
    let first_deadline = Instant::now() + attempt.first_acquire_window;
    let acquired = loop {
        let acquiring = timeout_at(first_deadline, pool.acquire());
        let acquired = if attempt.identity_rejection_is_terminal {
            tokio::select! {
                acquired = acquiring => acquired,
                error = rejection.rejected() => Ok(Err(error)),
            }
        } else {
            acquiring.await
        };
        match acquired {
            Ok(Err(sqlx::Error::PoolTimedOut)) if Instant::now() < first_deadline => {}
            Ok(acquired) => break acquired,
            Err(_) => break Err(sqlx::Error::PoolTimedOut),
        }
    };
    let result = acquired.map(|connection| {
        drop(connection);
        pool
    });
    Ok((result, observation))
}

async fn verify_identity(pool: &MySqlPool, directory: &Path, identity: &Identity) -> Result<()> {
    verify_identity_until(
        pool,
        directory,
        identity,
        Instant::now() + ORDINARY_POOL_WINDOW,
    )
    .await
}

async fn verify_identity_until(
    pool: &MySqlPool,
    directory: &Path,
    identity: &Identity,
    deadline: Instant,
) -> Result<()> {
    timeout_at(deadline, async {
        let datadir: String = sqlx::query_scalar("SELECT @@datadir")
            .fetch_one(pool)
            .await?;
        ensure!(
            same_directory(Path::new(&datadir), &directory.join("data"))?,
            "memory server data directory mismatch"
        );
        let row =
            sqlx::query("SELECT instance_id, project_scope FROM kuru_instance WHERE singleton = 1")
                .fetch_one(pool)
                .await?;
        ensure!(
            row.try_get::<String, _>("instance_id")? == identity.instance
                && row.try_get::<String, _>("project_scope")? == identity.project_scope,
            "memory SQL project/instance identity mismatch"
        );
        Ok(())
    })
    .await
    .context("memory identity query deadline exceeded")?
}

async fn live_endpoint(
    directory: &Path,
    identity: &Identity,
    read_only: bool,
) -> Result<Option<Endpoint>> {
    let Some(endpoint): Option<Endpoint> = read_record(&directory.join("endpoint.json"))? else {
        return Ok(None);
    };
    ensure!(
        identity.initialized && endpoint.instance == identity.instance,
        "memory endpoint is not initialized for this instance"
    );
    match timeout(
        Duration::from_millis(250),
        TcpStream::connect(("127.0.0.1", endpoint.port)),
    )
    .await
    {
        Ok(Ok(stream)) => drop(stream),
        _ => return Ok(None),
    }
    let (result, observation) = connect_pool_attempt(
        identity,
        &endpoint,
        directory,
        "main",
        read_only,
        1,
        PoolAttemptOptions::ordinary(),
    )
    .await
    .context("authenticate published memory endpoint")?;
    let pool = match result {
        Ok(pool) => pool,
        Err(error) if observation.is_pre_callback_connection_reset(&error) => return Ok(None),
        Err(error) => {
            return Err(connection_error(error, &observation))
                .context("authenticate published memory endpoint");
        }
    };
    let verified = verify_identity(&pool, directory, identity)
        .await
        .context("verify published memory endpoint identity");
    pool.close().await;
    verified?;
    Ok(Some(endpoint))
}

async fn read_frame<R: AsyncRead + Unpin, T: DeserializeOwned>(reader: &mut R) -> Result<T> {
    let length = reader
        .read_u32()
        .await
        .context("memory supervisor frame header")? as usize;
    ensure!(
        length > 0 && length <= RECORD_LIMIT,
        "invalid memory supervisor frame length"
    );
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .await
        .context("memory supervisor frame body")?;
    serde_json::from_slice(&bytes).context("invalid memory supervisor frame")
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= RECORD_LIMIT,
        "memory supervisor frame exceeds limit"
    );
    writer.write_u32(bytes.len().try_into()?).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Internal entrypoint shared by the application and package test helper.
pub async fn supervisor_entry() -> Result<()> {
    #[cfg(unix)]
    {
        let mut input =
            pipe::Receiver::from_owned_fd(std::io::stdin().as_fd().try_clone_to_owned()?)?;
        let mut output =
            pipe::Sender::from_owned_fd(std::io::stdout().as_fd().try_clone_to_owned()?)?;
        let request: Request = timeout(Duration::from_secs(5), read_frame(&mut input))
            .await
            .context("memory supervisor configuration deadline exceeded")??;
        supervisor_request(request, &mut input, &mut output).await
    }
    #[cfg(windows)]
    {
        let address = std::env::args_os()
            .nth(2)
            .context("Windows memory supervisor needs its private rendezvous address")?;
        let started = Instant::now();
        let mut channel = pipe::connect(&address, Duration::from_secs(5)).await?;
        let request = timeout(
            Duration::from_secs(5).saturating_sub(started.elapsed()),
            read_frame::<_, Request>(&mut channel),
        )
        .await
        .context("memory supervisor configuration deadline exceeded")??;
        let (mut input, mut output) = tokio::io::split(channel);
        let result = supervisor_request(request, &mut input, &mut output).await;
        let mut channel = input.unsplit(output);
        let closed = channel.close(KILL_GRACE).await;
        result?;
        closed?;
        Ok(())
    }
}

async fn supervisor_request<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    request: Request,
    input: &mut R,
    output: &mut W,
) -> Result<()> {
    if let Err(error) = supervise(request, input, output).await {
        // No credentials are ever serialized in the response. SQL bootstrap
        // credential-setting failures are deliberately reported without SQL text.
        let message = format!("{error:#}");
        let _ = timeout(KILL_GRACE, write_frame(output, &Response::Failed(message))).await;
        return Err(error);
    }
    Ok(())
}

async fn supervise<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    request: Request,
    input: &mut R,
    output: &mut W,
) -> Result<()> {
    supervise_with_port_hook(request, input, output, |_| Ok(())).await
}

/// At most this many owned Dolt startup attempts may run, all inside the one
/// original startup deadline, and only while Dolt itself reports that the
/// freshly selected loopback port was taken before it could bind.
const OWNED_START_ATTEMPTS: usize = 3;

async fn supervise_with_port_hook<
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnMut(u16) -> Result<()>,
>(
    request: Request,
    input: &mut R,
    output: &mut W,
    mut port_selected: F,
) -> Result<()> {
    ensure!(
        request.timeout_millis > 0 && request.timeout_millis <= 300_000,
        "invalid supervisor startup timeout"
    );
    prepare_directory(&request.directory, request.read_only)?;
    ensure!(
        fs::canonicalize(&request.directory)? == request.directory,
        "memory directory must be canonical"
    );
    let deadline = Instant::now() + Duration::from_millis(request.timeout_millis);
    let lease = LifecycleLease::new(&request.directory, request.lifecycle_root.as_deref())?;
    loop {
        lease.verify()?;
        match lease.lock.try_lock() {
            Ok(()) => {
                lease.verify()?;
                break;
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                if request.read_only
                    && let Some(identity) =
                        load_identity(&request.directory, &request.project_scope)?
                    && let Some(endpoint) =
                        live_endpoint(&request.directory, &identity, request.read_only).await?
                {
                    write_frame(
                        output,
                        &Response::Ready {
                            endpoint,
                            owned: false,
                        },
                    )
                    .await?;
                    return Ok(());
                }
                ensure!(
                    Instant::now() < deadline,
                    "memory lifecycle lock is held; refusing takeover"
                );
                let mut byte = [0];
                tokio::select! {
                    result = input.read(&mut byte) => { result?; bail!("memory parent closed during lifecycle acquisition"); }
                    () = sleep(Duration::from_millis(25)) => {}
                }
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
    }
    let mut identity = match load_identity(&request.directory, &request.project_scope)? {
        Some(identity) => identity,
        None => {
            ensure!(!request.read_only, "memory has not been initialized");
            ensure!(
                !request.directory.join("data").exists(),
                "unrecognized memory data without identity"
            );
            let identity = Identity {
                version: 1,
                instance: Uuid::new_v4().to_string(),
                project_scope: request.project_scope.clone(),
                password: secret(),
                reader_password: secret(),
                initialized: false,
            };
            write_record(&request.directory.join("identity.json"), &identity)?;
            identity
        }
    };
    ensure!(
        !request.read_only || identity.initialized,
        "memory initialization is incomplete"
    );
    for name in ["data", "config", "home"] {
        private_directory(&request.directory.join(name))?;
    }
    crate::provision::prepare_private_home(&request.directory.join("home"))?;
    let mut signals = ShutdownSignals::new()?;
    let mut byte = [0];
    let mut attempt = 0usize;
    loop {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("reserve a private loopback port for Dolt")?;
        let port = listener.local_addr()?.port();
        ensure!(port >= 1024, "unsupported allocated Dolt port");
        drop(listener);
        port_selected(port)?;
        let endpoint = Endpoint {
            instance: identity.instance.clone(),
            port,
        };
        let yaml = server_yaml(
            &request.directory,
            port,
            Duration::from_millis(request.timeout_millis),
        )?;
        let config_path = request.directory.join("server.yaml");
        write_private(&config_path, yaml.as_bytes())?;
        let mut child = crate::engine::spawn(
            &request.binary,
            &request.directory.join("home"),
            &request.directory,
            vec![
                "sql-server".into(),
                "--config".into(),
                config_path.into_os_string(),
            ],
            vec![
                (
                    "DOLT_ROOT_PASSWORD".into(),
                    identity.password.clone().into(),
                ),
                ("DOLT_ROOT_HOST".into(), "localhost".into()),
            ],
            true,
        )
        .await?;
        let log = Arc::new(Mutex::new(Vec::new()));
        // Test-support measurement only: a live mirror outliving the fixture.
        #[cfg(any(test, feature = "test-support"))]
        let (mirror, stdout_mirror, stderr_mirror) = {
            let mirror = crate::test_support::lifecycle_trace::DoltMirror::open(
                &request.directory,
                vec![identity.password.clone(), identity.reader_password.clone()],
            );
            (mirror.clone(), mirror.clone(), mirror)
        };
        #[cfg(not(any(test, feature = "test-support")))]
        let (stdout_mirror, stderr_mirror): (LogMirror, LogMirror) = ((), ());
        let stdout = tokio::spawn(drain(
            child.stdout().context("Dolt stdout missing")?,
            log.clone(),
            stdout_mirror,
        ));
        let stderr = tokio::spawn(drain(
            child.stderr().context("Dolt stderr missing")?,
            log.clone(),
            stderr_mirror,
        ));
        let run_result = async {
        tokio::select! {
            ready = start_database(&mut child, &request.directory, &mut identity, &endpoint, deadline) => ready,
            result = input.read(&mut byte) => { result?; Err(anyhow!("memory parent closed during startup")) },
            _ = signals.recv() => Err(anyhow!("memory supervisor terminated during startup")),
        }?;
        write_record(&request.directory.join("endpoint.json"), &endpoint)?;
        write_frame(output, &Response::Ready { endpoint: endpoint.clone(), owned: true }).await?;
        tokio::select! {
            status = child.wait() => { let status = status?; ensure!(status.success(), "Dolt exited unexpectedly ({status})"); Ok(()) },
            result = input.read(&mut byte) => { result?; Ok(()) },
            _ = signals.recv() => Ok(()),
        }
        }.await;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(mirror) = &mirror {
            mirror.note(
                "stop_begin",
                format_args!(
                    "run_ok={} dir_exists={}",
                    run_result.is_ok(),
                    crate::test_support::lifecycle_trace::exists(&request.directory)
                ),
            );
        }
        let stopped = stop_child(&mut child).await;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(mirror) = &mirror {
            mirror.note(
                "child_exit",
                format_args!(
                    "stopped={:?} dir_exists={}",
                    stopped.as_ref().map_err(|error| format!("{error:#}")),
                    crate::test_support::lifecycle_trace::exists(&request.directory)
                ),
            );
        }
        if stopped.is_err() {
            // The parent has its own bounded close deadline. A failure to reap
            // cannot make the directory safe to move: retain this supervisor and
            // its lifecycle lease until the actual owned process has terminated.
            eprintln!("Dolt cleanup is delayed; retaining the memory lifecycle lease");
            observe_dolt(&mut child, Child::try_wait).await;
        }
        let drained = timeout(Duration::from_secs(1), async {
            stdout.await?;
            stderr.await?;
            Ok::<(), tokio::task::JoinError>(())
        })
        .await;
        let mut diagnostic = String::from_utf8_lossy(&log.lock().await)
            .replace(&identity.password, "[redacted]")
            .replace(&identity.reader_password, "[redacted]");
        match &stopped {
            Ok(outcome) => {
                diagnostic.push_str(&format!("\nKuru engine shutdown: {outcome:?}\n"));
            }
            Err(_) => diagnostic.push_str("\nKuru engine shutdown: observed after cleanup error\n"),
        }
        let written = write_private(&request.directory.join("server.log"), diagnostic.as_bytes());
        #[cfg(any(test, feature = "test-support"))]
        if let Some(mirror) = &mirror {
            mirror.note("server_log_write", format_args!("ok={}", written.is_ok()));
        }
        written?;
        if let Some(published) = read_record::<Endpoint>(&request.directory.join("endpoint.json"))?
            && published.instance == endpoint.instance
            && published.port == endpoint.port
        {
            retire_endpoint(&request.directory)?;
        }
        stopped?;
        match run_result {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < OWNED_START_ATTEMPTS
                    && error.downcast_ref::<DoltPrematureExit>().is_some()
                    && drained.is_ok_and(|result| result.is_ok())
                    && diagnostic.lines().any(|line| {
                        line.trim_end_matches('\r') == format!("Port {port} already in use.")
                    })
                    && Instant::now() < deadline =>
            {
                // A parent close or termination may have become ready while
                // the failed child was being reaped. It must win over retry.
                tokio::select! {
                    biased;
                    result = input.read(&mut byte) => {
                        result?;
                        bail!("memory parent closed during startup");
                    }
                    _ = signals.recv() => bail!("memory supervisor terminated during startup"),
                    () = std::future::ready(()) => {}
                }
                attempt += 1;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Dolt startup/lifetime failed; private diagnostics: {}",
                        request.directory.join("server.log").display()
                    )
                });
            }
        }
    }
    // `lease` is released only after child cleanup and endpoint retirement.
}

fn same_directory(actual: &Path, expected: &Path) -> Result<bool> {
    Ok(files::directory(actual)?.identity() == files::directory(expected)?.identity())
}

async fn observe_dolt(
    child: &mut Child,
    mut status: impl FnMut(&mut Child) -> std::io::Result<Option<std::process::ExitStatus>>,
) {
    // The caller retains the lifecycle lease throughout this observation.
    // NativeChild only reports exit once its owned Job has no live processes.
    loop {
        if matches!(status(child), Ok(Some(_))) {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
}

fn retire_endpoint(directory: &Path) -> Result<()> {
    let parent = files::directory(directory)?;
    let stage = files::ensure_private_directory(&directory.join("staging"))?;
    let source = std::ffi::OsStr::new("endpoint.json");
    let file = parent.read(source)?;
    let retired = format!("endpoint-{}.retired", Uuid::new_v4());
    stage.rename_file(
        &parent,
        source,
        &file,
        std::ffi::OsStr::new(&retired),
        kuru_platform::fs::Publication::New,
    )?;
    stage.remove_file(std::ffi::OsStr::new(&retired), file)?;
    Ok(())
}

#[cfg(unix)]
struct ShutdownSignals {
    termination: tokio::signal::unix::Signal,
    interrupt: tokio::signal::unix::Signal,
}
#[cfg(windows)]
struct ShutdownSignals {
    termination: tokio::signal::windows::CtrlBreak,
    interrupt: tokio::signal::windows::CtrlC,
}
impl ShutdownSignals {
    fn new() -> Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                termination: signal(SignalKind::terminate())?,
                interrupt: signal(SignalKind::interrupt())?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                termination: tokio::signal::windows::ctrl_break()?,
                interrupt: tokio::signal::windows::ctrl_c()?,
            })
        }
    }
    async fn recv(&mut self) {
        tokio::select! { _ = self.termination.recv() => {}, _ = self.interrupt.recv() => {} }
    }
}

fn server_yaml(directory: &Path, port: u16, startup_timeout: Duration) -> Result<String> {
    // Dolt also applies this listener value while its result iterator executes
    // SQL, including CREATE DATABASE. Do not silently cancel work before our
    // existing bootstrap/operation deadlines. Caller cancellation and owned
    // shutdown still determine when resources can be released.
    let read_timeout = startup_timeout.max(crate::store::QUERY_TIMEOUT).as_millis();
    // Dolt 2.3.3 does not derive @@datadir from data_dir. Initialize its
    // read-only SQL variable from the same canonical path for identity probes.
    let quoted = |name: &str| serde_json::to_string(&directory.join(name));
    Ok(format!(
        "log_level: warning\nlog_format: text\nbehavior:\n  autocommit: true\n  dolt_transaction_commit: false\n  event_scheduler: \"OFF\"\n  auto_gc_behavior:\n    enable: true\nlistener:\n  host: 127.0.0.1\n  port: {port}\n  max_connections: 32\n  max_connections_timeout_millis: 1000\n  read_timeout_millis: {read_timeout}\n  write_timeout_millis: 5000\n  allow_cleartext_passwords: false\ndata_dir: {}\ncfg_dir: {}\nprivilege_file: {}\nbranch_control_file: {}\nsystem_variables:\n  datadir: {}\n  secure_file_priv: {}\n",
        quoted("data")?,
        quoted("config")?,
        quoted("config/privileges.db")?,
        quoted("config/branch_control.db")?,
        quoted("data")?,
        quoted("file-operations-disabled")?
    ))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    files::write(path, bytes)
}

#[cfg(any(test, feature = "test-support"))]
type LogMirror = Option<Arc<crate::test_support::lifecycle_trace::DoltMirror>>;
#[cfg(not(any(test, feature = "test-support")))]
type LogMirror = ();

async fn drain<R: AsyncRead + Unpin>(mut reader: R, log: Arc<Mutex<Vec<u8>>>, mirror: LogMirror) {
    #[cfg(not(any(test, feature = "test-support")))]
    let () = mirror;
    let mut bytes = [0; 4096];
    while let Ok(length) = reader.read(&mut bytes).await {
        if length == 0 {
            break;
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(mirror) = &mirror {
            mirror.write(&bytes[..length]);
        }
        let mut log = log.lock().await;
        log.extend_from_slice(&bytes[..length]);
        if log.len() > LOG_LIMIT {
            let remove = log.len() - LOG_LIMIT;
            log.drain(..remove);
        }
    }
}

#[derive(Debug)]
struct DoltPrematureExit(std::process::ExitStatus);

impl std::fmt::Display for DoltPrematureExit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Dolt exited before readiness ({})", self.0)
    }
}

impl std::error::Error for DoltPrematureExit {}

async fn start_database(
    child: &mut Child,
    directory: &Path,
    identity: &mut Identity,
    endpoint: &Endpoint,
    deadline: Instant,
) -> Result<()> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(DoltPrematureExit(status).into());
        }
        ensure!(
            Instant::now() < deadline,
            "authenticated Dolt startup deadline exceeded"
        );
        let options = MySqlConnectOptions::new()
            .host("127.0.0.1")
            .port(endpoint.port)
            .username("root")
            .password(&identity.password)
            .ssl_mode(MySqlSslMode::Disabled);
        if let Ok(Ok(pool)) = timeout(
            Duration::from_millis(400),
            MySqlPoolOptions::new()
                .max_connections(1)
                .connect_with(options),
        )
        .await
        {
            let mut phase = "checking the bootstrap data directory";
            let initialized = timeout(
                deadline.saturating_duration_since(Instant::now()),
                initialize_database(&pool, directory, identity, &mut phase),
            )
            .await;
            pool.close().await;
            return initialized
                .with_context(|| {
                    format!("Dolt database bootstrap deadline exceeded while {phase}")
                })?
                .with_context(|| format!("Dolt database bootstrap failed while {phase}"));
        }
        sleep(Duration::from_millis(25)).await;
    }
}

async fn initialize_database(
    pool: &MySqlPool,
    directory: &Path,
    identity: &mut Identity,
    phase: &mut &'static str,
) -> Result<()> {
    let datadir: String = sqlx::query_scalar("SELECT @@datadir")
        .fetch_one(pool)
        .await?;
    ensure!(
        same_directory(Path::new(&datadir), &directory.join("data"))
            .with_context(|| format!("resolve Dolt bootstrap datadir {datadir:?}"))?,
        "Dolt bootstrap data directory mismatch"
    );
    if !identity.initialized {
        *phase = "creating the project database";
        sqlx::query("CREATE DATABASE IF NOT EXISTS kuru")
            .execute(pool)
            .await?;
        *phase = "creating the project identity table";
        sqlx::query("CREATE TABLE IF NOT EXISTS kuru.kuru_instance (singleton TINYINT PRIMARY KEY, instance_id VARCHAR(36) NOT NULL, project_scope TEXT NOT NULL)").execute(pool).await?;
        *phase = "initializing the project identity";
        let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kuru.kuru_instance")
            .fetch_one(pool)
            .await?;
        if existing == 0 {
            sqlx::query("INSERT INTO kuru.kuru_instance (singleton, instance_id, project_scope) VALUES (1, ?, ?)").bind(&identity.instance).bind(&identity.project_scope).execute(pool).await?;
        }
        // Dolt's CREATE USER parser rejects bind parameters. Only our generated
        // 64-character hex credential can enter this literal; never user input.
        ensure!(
            valid_secret(&identity.reader_password),
            "invalid private reader credential"
        );
        let credential_sql = format!(
            "CREATE USER IF NOT EXISTS 'kuru_reader'@'localhost' IDENTIFIED BY '{}'",
            identity.reader_password
        );
        *phase = "configuring the private reader account";
        sqlx::query(sqlx::AssertSqlSafe(credential_sql))
            .execute(pool)
            .await
            .map_err(|_| anyhow!("configuring private read-only memory account failed"))?;
        *phase = "granting private reader access";
        sqlx::query("GRANT SELECT ON kuru.* TO 'kuru_reader'@'localhost'")
            .execute(pool)
            .await?;
    }
    *phase = "verifying the project identity";
    let row = sqlx::query(
        "SELECT instance_id, project_scope FROM kuru.kuru_instance WHERE singleton = 1",
    )
    .fetch_one(pool)
    .await?;
    ensure!(
        row.try_get::<String, _>("instance_id")? == identity.instance
            && row.try_get::<String, _>("project_scope")? == identity.project_scope,
        "Dolt bootstrap project identity mismatch"
    );
    if !identity.initialized {
        *phase = "publishing the initialized identity";
        identity.initialized = true;
        write_record(&directory.join("identity.json"), identity)?;
    }
    Ok(())
}

async fn stop_child(child: &mut Child) -> Result<crate::engine::StopOutcome> {
    child.stop(CLOSE_GRACE, KILL_GRACE).await
}

#[cfg(test)]
mod stale_endpoint_tests {
    use super::*;

    #[tokio::test]
    async fn pre_callback_connection_reset_is_not_a_live_endpoint() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let directory = root.path().join("memory");
        private_directory(&directory).context("create private endpoint fixture")?;
        let identity = Identity {
            version: 1,
            instance: Uuid::new_v4().to_string(),
            project_scope: "project/test".into(),
            password: secret(),
            reader_password: secret(),
            initialized: true,
        };
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind reset listener")?;
        let endpoint = Endpoint {
            instance: identity.instance.clone(),
            port: listener.local_addr()?.port(),
        };
        write_record(&directory.join("endpoint.json"), &endpoint)
            .context("write stale endpoint record")?;
        let mut observed = tokio::spawn(async move {
            let (probe, _) = listener.accept().await?;
            drop(probe);
            let (stream, _) = listener.accept().await?;
            stream.set_zero_linger()?;
            drop(stream);
            Ok::<_, std::io::Error>(())
        });

        let endpoint = live_endpoint(&directory, &identity, false).await;
        match timeout(Duration::from_secs(3), &mut observed).await {
            Ok(result) => result??,
            Err(error) => {
                observed.abort();
                let _ = observed.await;
                return Err(error)
                    .context("reset listener did not observe the raw probe and SQL connection");
            }
        }
        assert!(endpoint?.is_none());
        Ok(())
    }
}

#[cfg(test)]
#[path = "server/branch_procedure_tests.rs"]
mod branch_procedure_tests;
#[cfg(test)]
#[path = "server/startup_budget_tests.rs"]
mod startup_budget_tests;
#[cfg(all(test, unix))]
#[path = "server_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "server/vanished_entry_tests.rs"]
mod vanished_entry_tests;
#[cfg(all(test, windows))]
#[path = "server/windows_tests.rs"]
mod windows_tests;

#[cfg(all(windows, any(test, feature = "test-support")))]
pub(crate) mod windows_fixture;
