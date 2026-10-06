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
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{engine::Child, files, pool::MemoryPool};
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
    sync::{Mutex, OwnedMutexGuard},
    time::{Instant, sleep, timeout, timeout_at},
};
use uuid::Uuid;

/// The floor of an opening server's pool attempt window and identity query,
/// and the attach probe's window. Once memory is open, pool creation and
/// every acquisition are bounded by the budget of the work they serve
/// instead, with a pool ceiling of `QUERY_TIMEOUT`; the pool's 2 s
/// slow-acquire record (`pool::SLOW_ACQUIRE_THRESHOLD`) only logs.
pub(crate) const OPENING_POOL_FLOOR: Duration = Duration::from_secs(2);
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

/// The instance a store template's identity row holds on `main` and on the
/// usage branch until a copy adopts it: the nil UUID, which no generated
/// instance (a random version-4 UUID) can equal. It is also the instance of a
/// template build's own identity record.
pub(crate) const TEMPLATE_INSTANCE: &str = "00000000-0000-0000-0000-000000000000";
/// The project scope a store template's identity row holds until adoption:
/// `project/` and the SHA-256 of `kuru-memory-store-template-scope-v1`. It
/// passes every scope check a project identity does and names no project.
pub(crate) const TEMPLATE_SCOPE: &str =
    "project/611dd11842f30482f942494b150ce33784d70efdf91d47c7a9f498d95c81662d";
/// Bound on the `template` key an identity record may carry. The template
/// cache names directories after the key (`.rejected-<key>-<uuid>` is the
/// longest), so a key of this length still fits one 255-byte path component.
const TEMPLATE_KEY_LIMIT: usize = 128;
/// The permanent usage branch, adopted before `main`.
pub(crate) const USAGE_DATABASE: &str = "kuru/kuru_usage_v1";
/// The message of the adoption commit on each ref; [`crate::store::AUTHOR`]
/// is its author.
pub(crate) const ADOPTION_MESSAGE: &str = "Adopt Kuru memory template";

/// The store template key this supervisor was compiled to trust: the
/// template cache's key over the schema, the engine and the creation
/// statements (`store::creation_template`). A stage whose pending identity
/// names another key is refused before any write, and the refusal is not a
/// verdict against its bytes.
pub(crate) fn compiled_template_key() -> &'static str {
    crate::store::creation_template::compiled_key()
}

/// The bootstrap statements a first engine start runs on a new store: its
/// database, its identity table and, for a store that has none, its identity
/// row. Every one shapes the bytes of a new store's `data/`, so the store
/// template key (`store::creation_template`) covers them; call sites use
/// these constants, never inline text.
pub(crate) const BOOTSTRAP_CREATE_DATABASE: &str = "CREATE DATABASE IF NOT EXISTS kuru";
pub(crate) const BOOTSTRAP_CREATE_IDENTITY: &str = "CREATE TABLE IF NOT EXISTS kuru.kuru_instance (singleton TINYINT PRIMARY KEY, instance_id VARCHAR(36) NOT NULL, project_scope TEXT NOT NULL)";
pub(crate) const BOOTSTRAP_COUNT_IDENTITY: &str = "SELECT COUNT(*) FROM kuru.kuru_instance";
pub(crate) const BOOTSTRAP_INSERT_IDENTITY: &str =
    "INSERT INTO kuru.kuru_instance (singleton, instance_id, project_scope) VALUES (1, ?, ?)";
/// The reader account statement around its generated secret. The secret is
/// per store and never keyed: `config/`, where the account lives, is never
/// part of a template.
pub(crate) const BOOTSTRAP_READER_ACCOUNT: [&str; 2] = [
    "CREATE USER IF NOT EXISTS 'kuru_reader'@'localhost' IDENTIFIED BY '",
    "'",
];
pub(crate) const BOOTSTRAP_READER_GRANT: &str =
    "GRANT SELECT ON kuru.* TO 'kuru_reader'@'localhost'";
/// The `behavior:` block of every engine's `server.yaml`: the SQL settings
/// under which bootstrap, initialization and migrations write a store.
pub(crate) const SERVER_BEHAVIOR: &str = "behavior:\n  autocommit: true\n  dolt_transaction_commit: false\n  event_scheduler: \"OFF\"\n  auto_gc_behavior:\n    enable: true\n";

/// A completed comparison found a store template's bytes other than this
/// build expects: a placeholder row that is missing, foreign or repeated, a
/// guarded rewrite that changed a count of rows other than one, an adopted
/// row that differs from the new identity, or a template shape other than
/// the compiled one.
///
/// Only this error is a verdict against a template. Engine exits, SQL and
/// transport errors, I/O errors, deadlines, authentication failures and a
/// template key other than the compiled one say nothing about the bytes and
/// are never wrapped in it. The supervisor reports it as
/// [`Response::TemplateRejected`], and the client turns that response back
/// into this type, so a caller finds it with [`TemplateVerdict::find`] on
/// either side of the process boundary.
#[derive(Debug)]
pub(crate) struct TemplateVerdict(String);

impl TemplateVerdict {
    pub(crate) fn new(reason: impl Into<String>) -> Self {
        Self(reason.into())
    }

    /// The verdict anywhere in `error`'s chain of causes and contexts.
    pub(crate) fn find(error: &anyhow::Error) -> Option<&Self> {
        error.chain().find_map(|cause| cause.downcast_ref::<Self>())
    }
}

impl fmt::Display for TemplateVerdict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TemplateVerdict {}

/// Worst-case owned close, as bounded by `close_pools_and_owner`: the first
/// graceful pool drain, the Windows lifetime close, the supervisor reap
/// allowance in `finish_owner`, and the post-reap pool drain.
pub(crate) fn close_budget() -> Duration {
    CLOSE_GRACE
        .saturating_add(KILL_GRACE)
        .saturating_add(SUPERVISOR_REAP_ALLOWANCE)
        .saturating_add(CLOSE_GRACE)
}

#[derive(Clone, Debug)]
pub struct ServerOptions {
    /// Existing-client startup is checked again by the owning supervisor
    /// while its lifecycle lease is held, before identity or database creation.
    pub expected_instance: Option<String>,
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
    /// The progress counter of the open this engine start belongs to, if it
    /// counts. The start advances it at its milestones and marks it failing
    /// before it reaps an engine whose failure fails that open.
    pub ticks: Option<crate::progress::OpenTicks>,
}

#[derive(Clone)]
pub struct Server(Arc<ServerInner>);

struct ServerInner {
    directory: PathBuf,
    identity: Identity,
    endpoint: Endpoint,
    read_only: bool,
    pools: Mutex<BTreeMap<String, Weak<MemoryPool>>>,
    pool_admission: Mutex<BTreeMap<String, Weak<Mutex<()>>>>,
    /// The deadline that bounded this owned start's readiness and first
    /// authenticated probe. Pools the store opening requests from this server
    /// continue that one startup operation until the store reports ready.
    opening_deadline: StdMutex<Option<Instant>>,
    #[cfg(test)]
    candidate_wait_observer: Mutex<Option<oneshot::Sender<()>>>,
    #[cfg(test)]
    next_pool_probe_delay: StdMutex<Option<(Duration, Arc<AtomicBool>)>>,
    #[cfg(test)]
    next_pool_first_release_hold: StdMutex<Option<tokio::sync::oneshot::Sender<ConnectionGate>>>,
    #[cfg(test)]
    next_pool_authentication_gate: StdMutex<Option<tokio::sync::oneshot::Sender<ConnectionGate>>>,
    /// Every branch or revision a caller asked [`Server::pool`] for, in order.
    #[cfg(test)]
    pool_requests: StdMutex<Vec<String>>,
    owner: Mutex<Option<Owner>>,
    reap_guard: Arc<StdMutex<Option<File>>>,
    closed: AtomicBool,
    /// The progress counter of the open that started this server, if any.
    ticks: Option<crate::progress::OpenTicks>,
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
            // Released after the reap, explicitly: a startup lock or a
            // template build's key lock must not stay held by a sibling's
            // child that duplicated it between fork and exec.
            if let Some(guard) = guard {
                files::release_lock(guard);
            }
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
    /// The store template key a stage copied from a template was created
    /// under, or a template build's own key. Kept for the store's life: it
    /// marks a template-born stage for recovery, and while `initialized` is
    /// false it must equal [`compiled_template_key`]. A store created
    /// directly or by import has none and never serializes the field, so its
    /// record keeps its bytes; a binary that predates the field fails closed
    /// on a record carrying it (`deny_unknown_fields`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    template: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_instance: Option<String>,
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
    Ready {
        endpoint: Endpoint,
        owned: bool,
    },
    Failed(String),
    /// Startup failed on a [`TemplateVerdict`]. Every other failure is
    /// [`Response::Failed`]. Only a same-build client creates template
    /// stages, and a supervisor of another build refuses them on the key
    /// before this can be sent, so an older client never receives it.
    TemplateRejected(String),
}

async fn drain_closed_pools(pools: &[Arc<MemoryPool>]) {
    // `Pool::close` marks its pool closed before returning the future. Build
    // all futures first so one slow MySQL QUIT cannot leave a sibling branch
    // pool admitting work while the exact owner is being reaped.
    let drains = pools.iter().map(|pool| pool.close()).collect::<Vec<_>>();
    futures::future::join_all(drains).await;
}

async fn close_pools_and_owner(pools: &[Arc<MemoryPool>], owner: Option<Owner>) -> Result<()> {
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
        prepare_directory(
            &options.directory,
            options.read_only || options.expected_instance.is_some(),
        )
        .context("prepare private memory directory before startup")?;
        let directory = fs::canonicalize(&options.directory)
            .context("resolve private memory directory before startup")?;
        LifecycleLease::validate_root(&directory, options.lifecycle_root.as_deref())
            .context("validate memory lifecycle directory before startup")?;
        if let Some(expected) = &options.expected_instance {
            require_existing_instance(&directory, &options.project_scope, expected)?;
        }
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
                    options.ticks,
                ));
            }
        } else {
            ensure!(!options.read_only, "memory has not been initialized");
        }

        let request = Request {
            expected_instance: options.expected_instance.clone(),
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
            // Test builds hold the spawn gate across child creation; see
            // `crate::spawn_gate`.
            #[cfg(test)]
            let creation = crate::spawn_gate::child_creation().await;
            #[cfg(any(test, feature = "test-support"))]
            let program = PathBuf::from(command.get_program());
            let child = kuru_platform::unix::spawn_piped(command)
                .context("start memory lifetime supervisor")?;
            let spawned = Instant::now();
            // The test gate guards this process's child creation, not the
            // supervisor's later startup or the delayed authentication probe.
            #[cfg(test)]
            drop(creation);
            // Test-support only: the supervisor leaves this process group, so
            // a coverage partition names the test behind it if it outlives
            // that test.
            #[cfg(any(test, feature = "test-support"))]
            crate::test_support::spawn_ledger::record(
                child.id(),
                &program,
                crate::test_support::spawn_ledger::DOLT_SUPERVISOR,
            );
            crate::open_timeline::stamp(crate::open_timeline::Event::SupervisorSpawned);
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
            let step = ReadinessStep::new(ReadinessPart::Write);
            let response = timeout_at(startup_deadline, async {
                write_frame(owner.lifetime.as_mut().expect("owned lifetime"), &request)
                    .await
                    .context("send memory supervisor startup request")?;
                step.set(ReadinessPart::Read);
                read_frame::<_, Response>(&mut output)
                    .await
                    .context("read memory supervisor readiness response")
            })
            .await
            .map_err(|elapsed| readiness_deadline(elapsed.into(), step.get(), spawned))
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
            let spawned = Instant::now();
            // Test-support only: name the test behind a supervisor that
            // outlives it.
            #[cfg(any(test, feature = "test-support"))]
            crate::test_support::spawn_ledger::record(
                child.id(),
                &options.supervisor,
                crate::test_support::spawn_ledger::DOLT_SUPERVISOR,
            );
            crate::open_timeline::stamp(crate::open_timeline::Event::SupervisorSpawned);
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
            let step = ReadinessStep::new(ReadinessPart::Accept);
            let response = timeout_at(startup_deadline, async {
                // The accept's own timer runs to the same deadline. When it
                // fires first, its timeout is this deadline, reported as one.
                let accepted = accept.await.map_err(|error| {
                    match readiness_deadline_part(&error, Instant::now(), startup_deadline) {
                        Some(part) => readiness_deadline(error.into(), part, spawned),
                        None => anyhow::Error::new(error)
                            .context("accept memory supervisor private channel"),
                    }
                })?;
                crate::open_timeline::stamp(crate::open_timeline::Event::ChannelAccepted);
                owner.lifetime = Some(accepted);
                let channel = owner.lifetime.as_mut().expect("owned lifetime");
                step.set(ReadinessPart::Write);
                write_frame(channel, &request)
                    .await
                    .context("send memory supervisor startup request")?;
                step.set(ReadinessPart::Read);
                read_frame::<_, Response>(channel)
                    .await
                    .context("read memory supervisor readiness response")
            })
            .await
            .map_err(|elapsed| readiness_deadline(elapsed.into(), step.get(), spawned))
            .and_then(|response| response);
            (owner, response)
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return Err(startup_failure(&mut owner, error, options.ticks.as_ref()).await);
            }
        };
        let (endpoint, owned) = match response {
            Response::Ready { endpoint, owned } => {
                crate::progress::milestone(
                    options.ticks.as_ref(),
                    crate::open_timeline::Event::SupervisorReady,
                );
                if !owned && !options.read_only {
                    return Err(startup_failure(
                        &mut owner,
                        anyhow!("writable memory requires an owned supervisor lifetime"),
                        options.ticks.as_ref(),
                    )
                    .await);
                }
                (endpoint, owned)
            }
            Response::Failed(message) => {
                return Err(startup_failure(
                    &mut owner,
                    anyhow!("memory server startup failed: {message}"),
                    options.ticks.as_ref(),
                )
                .await);
            }
            Response::TemplateRejected(message) => {
                return Err(startup_failure(
                    &mut owner,
                    TemplateVerdict::new(format!(
                        "memory server startup rejected the store template: {message}"
                    ))
                    .into(),
                    options.ticks.as_ref(),
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
                        first_acquire_window: Some(remaining),
                        identity_rejection_is_terminal: true,
                        _test_probe_delay: _initial_probe_delay,
                        #[cfg(test)]
                        _test_first_release_hold: None,
                        #[cfg(test)]
                        _test_authentication_gate: None,
                    },
                ),
            )
            .await
            .context("post-readiness memory authentication deadline exceeded")?
            .context("authenticate post-readiness memory connection")?
            .0;
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
            Ok(identity) => {
                crate::progress::milestone(
                    options.ticks.as_ref(),
                    crate::open_timeline::Event::ProbeVerified,
                );
                identity
            }
            Err(error) => {
                return Err(startup_failure(&mut owner, error, options.ticks.as_ref()).await);
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
            options.ticks,
        ))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one constructor for the owned and borrowed starts"
    )]
    fn new(
        directory: PathBuf,
        identity: Identity,
        endpoint: Endpoint,
        read_only: bool,
        owner: Option<Owner>,
        reap_guard: Arc<StdMutex<Option<File>>>,
        opening_deadline: Option<Instant>,
        ticks: Option<crate::progress::OpenTicks>,
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
            #[cfg(test)]
            next_pool_first_release_hold: StdMutex::new(None),
            #[cfg(test)]
            next_pool_authentication_gate: StdMutex::new(None),
            #[cfg(test)]
            pool_requests: StdMutex::new(Vec::new()),
            owner: Mutex::new(owner),
            reap_guard,
            closed: AtomicBool::new(false),
            ticks,
        }))
    }

    /// Advance the count of the open that started this server: one
    /// completed bounded unit of that open's work, such as a migration step.
    pub(crate) fn advance_open(&self) {
        if let Some(ticks) = &self.0.ticks {
            ticks.advance();
        }
    }

    /// Stamp `event` and advance the count of the open that started this
    /// server.
    pub(crate) fn open_milestone(&self, event: crate::open_timeline::Event) {
        crate::progress::milestone(self.0.ticks.as_ref(), event);
    }

    /// Mark the open that started this server failing with `error`. Call
    /// only immediately before closing this server for a failure that the
    /// open then returns.
    pub(crate) fn mark_open_failing(&self, error: &anyhow::Error) {
        if let Some(ticks) = &self.0.ticks {
            ticks.mark_failing_with(self.failure_reason(error));
        }
    }

    /// `error`'s own text with this server's identity secrets replaced.
    pub(crate) fn failure_reason(&self, error: &anyhow::Error) -> String {
        redact_identity(format!("{error:#}"), &self.0.identity)
    }

    pub async fn pool(&self, branch: &str) -> Result<Arc<MemoryPool>> {
        #[cfg(test)]
        self.0
            .pool_requests
            .lock()
            .expect("pool request log lock")
            .push(branch.to_owned());
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
        // Read once: the opening phase can end while this pool is created.
        let pool = if let Some(window) = self.pool_attempt_window() {
            // While opening, the attempt continues the startup deadline and
            // verification takes its own window, never less than the floor.
            let pool = self.create_pool(branch, Some(window)).await?;
            verify_identity_until(
                &pool,
                &self.0.directory,
                &self.0.identity,
                Instant::now() + self.pool_attempt_window().unwrap_or(OPENING_POOL_FLOOR),
            )
            .await
            .context("verify memory branch pool identity")?;
            pool
        } else {
            // Once open, the first acquire, the first release and identity
            // verification share one creation budget, nested in any
            // enclosing budget scope. Its expiry fails the creation and
            // retains no pool; nothing is retried.
            let deadline = Instant::now() + crate::store::QUERY_TIMEOUT;
            crate::pool::within_until(deadline, async {
                let pool = self.create_pool(branch, None).await?;
                verify_identity_until(&pool, &self.0.directory, &self.0.identity, deadline)
                    .await
                    .context("verify memory branch pool identity")?;
                Ok::<_, anyhow::Error>(pool)
            })
            .await
            .context("memory branch pool creation budget elapsed")??
        };
        let pool = Arc::new(pool);
        pools.insert(branch.to_owned(), Arc::downgrade(&pool));
        Ok(pool)
    }

    /// Authenticate a new pool for `branch` with its first connection idle
    /// again. The identity queries that follow release inline through the
    /// funnel, so they reuse that one session.
    async fn create_pool(
        &self,
        branch: &str,
        first_acquire_window: Option<Duration>,
    ) -> Result<MemoryPool> {
        let (pool, observation) = connect_pool_with_timeout(
            &self.0.identity,
            &self.0.endpoint,
            &self.0.directory,
            branch,
            self.0.read_only,
            4,
            self.pool_attempt(first_acquire_window),
        )
        .await
        .context("authenticate memory branch pool")?;
        Ok(MemoryPool::new(pool, branch, observation))
    }

    /// The connection observation of the live pool for `branch`.
    #[cfg(test)]
    pub(crate) async fn pool_observation(&self, branch: &str) -> Option<ConnectionObservation> {
        self.0
            .pools
            .lock()
            .await
            .get(branch)
            .and_then(Weak::upgrade)
            .map(|pool| pool.observation().clone())
    }

    /// End the opening phase once the store that started this server is
    /// ready. Later pools are created under one creation budget.
    pub(crate) fn finish_opening(&self) {
        *self
            .0
            .opening_deadline
            .lock()
            .expect("opening deadline lock") = None;
    }

    pub(crate) fn opening_deadline(&self) -> Option<Instant> {
        *self
            .0
            .opening_deadline
            .lock()
            .expect("opening deadline lock")
    }

    /// While opening, the remaining startup deadline that bounded this
    /// server's own probe, never less than the opening floor: a slow but
    /// healthy start must not leave the next open-sequence pool with less
    /// than the floor. Once open, `None`: the creation budget bounds the
    /// attempt.
    fn pool_attempt_window(&self) -> Option<Duration> {
        self.opening_deadline().map(|deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .max(OPENING_POOL_FLOOR)
        })
    }

    fn pool_attempt(&self, first_acquire_window: Option<Duration>) -> PoolAttemptOptions {
        PoolAttemptOptions {
            // SQLx keeps this for every later acquire on the pool, so a pool
            // created while opening serves its later work with the same
            // ceiling: the statement budget. An acquisition inside a budget
            // scope is bounded by what remains of that budget.
            acquire_timeout: crate::store::QUERY_TIMEOUT,
            first_acquire_window,
            // No window may wait out an identity rejection; the same
            // endpoint cannot answer differently.
            identity_rejection_is_terminal: true,
            #[cfg(test)]
            _test_probe_delay: self
                .0
                .next_pool_probe_delay
                .lock()
                .expect("pool probe delay lock")
                .take(),
            #[cfg(not(test))]
            _test_probe_delay: None,
            #[cfg(test)]
            _test_first_release_hold: self
                .0
                .next_pool_first_release_hold
                .lock()
                .expect("pool first release hold lock")
                .take(),
            #[cfg(test)]
            _test_authentication_gate: self
                .0
                .next_pool_authentication_gate
                .lock()
                .expect("pool authentication gate lock")
                .take(),
        }
    }

    /// Hold the next pool's first connection release before SQLx's ping,
    /// until its attempt's deadline; the receiver gets the take-once gate.
    #[cfg(test)]
    pub(crate) fn hold_next_pool_first_release(
        &self,
    ) -> tokio::sync::oneshot::Receiver<ConnectionGate> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        *self
            .0
            .next_pool_first_release_hold
            .lock()
            .expect("pool first release hold lock") = Some(sender);
        receiver
    }

    /// Hold every new connection of the next pool at the start of its
    /// authentication until the gate the receiver gets is dropped. The next
    /// pool attempt takes this seam once; no time elapses inside the gate.
    #[cfg(test)]
    pub(crate) fn gate_next_pool_authentication(
        &self,
    ) -> tokio::sync::oneshot::Receiver<ConnectionGate> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        *self
            .0
            .next_pool_authentication_gate
            .lock()
            .expect("pool authentication gate lock") = Some(sender);
        receiver
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

    /// The branches and revisions [`Self::pool`] was asked for so far, in order,
    /// including requests a cached pool answered.
    #[cfg(test)]
    pub(crate) fn pool_requests(&self) -> Vec<String> {
        self.0
            .pool_requests
            .lock()
            .expect("pool request log lock")
            .clone()
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
        observer: &MemoryPool,
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
        observer: &MemoryPool,
        branch: &str,
        duration: Duration,
    ) -> Result<()> {
        let database = format!("kuru/{branch}");
        crate::pool::within(duration, async {
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

/// The step a supervisor start was in when its readiness deadline passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ReadinessPart {
    /// Only a Windows start accepts a private channel.
    #[cfg_attr(not(windows), allow(dead_code))]
    Accept,
    Write,
    Read,
}

impl std::fmt::Display for ReadinessPart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Accept => "the supervisor's private channel accept",
            Self::Write => "the startup request write",
            Self::Read => "the supervisor's Ready frame",
        })
    }
}

/// The current [`ReadinessPart`], set by the readiness steps and read after
/// the deadline dropped them. Atomic only so the start stays `Send`.
struct ReadinessStep(std::sync::atomic::AtomicU8);

impl ReadinessStep {
    fn new(part: ReadinessPart) -> Self {
        Self(std::sync::atomic::AtomicU8::new(part as u8))
    }

    fn set(&self, part: ReadinessPart) {
        self.0
            .store(part as u8, std::sync::atomic::Ordering::Relaxed);
    }

    fn get(&self) -> ReadinessPart {
        match self.0.load(std::sync::atomic::Ordering::Relaxed) {
            0 => ReadinessPart::Accept,
            1 => ReadinessPart::Write,
            _ => ReadinessPart::Read,
        }
    }
}

/// The supervisor readiness deadline, naming the step it passed in. The
/// outer cause text is unchanged; only the step between it and `cause` is
/// new. The one deadline instant, and every bound, stay as they are.
pub(crate) fn readiness_deadline(
    cause: anyhow::Error,
    part: ReadinessPart,
    spawned: Instant,
) -> anyhow::Error {
    let elapsed = spawned.elapsed().as_millis();
    cause
        .context(format!(
            "{part} had not completed {elapsed} ms after the supervisor was spawned"
        ))
        .context("memory supervisor readiness deadline exceeded")
}

/// A Windows accept fails with its own `TimedOut` when its timer, set to the
/// same deadline, fires before the outer one; at or after that deadline it is
/// the accept part of the deadline. `TimedOut` alone is not enough: other
/// failures map to it too.
#[cfg_attr(not(windows), allow(dead_code))]
fn readiness_deadline_part(
    error: &std::io::Error,
    now: Instant,
    deadline: Instant,
) -> Option<ReadinessPart> {
    (error.kind() == std::io::ErrorKind::TimedOut && now >= deadline)
        .then_some(ReadinessPart::Accept)
}

/// `text` without either connection secret of `identity`.
fn redact_identity(text: String, identity: &Identity) -> String {
    [&identity.password, &identity.reader_password]
        .into_iter()
        .filter(|secret| !secret.is_empty())
        .fold(text, |text, secret| {
            text.replace(secret.as_str(), "[redacted]")
        })
}

/// Reap a failed engine start. Every caller returns the error from the
/// start, and every start that carries an open's counter fails that open with
/// it, so the open is marked failing first, before the reap begins.
async fn startup_failure(
    owner: &mut Owner,
    error: anyhow::Error,
    ticks: Option<&crate::progress::OpenTicks>,
) -> anyhow::Error {
    if let Some(ticks) = ticks {
        ticks.mark_failing(&error);
    }
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
        validate_identity(identity)?;
    }
    Ok(identity)
}

/// The caller holds the store's startup lock through this check and open.
/// Missing or replaced identity is refusal, never permission to create it.
pub(crate) fn require_existing_instance(
    directory: &Path,
    project_scope: &str,
    expected: &str,
) -> Result<()> {
    ensure!(
        Uuid::parse_str(expected)?.to_string() == expected,
        "invalid expected memory instance"
    );
    let identity = load_identity(directory, project_scope)?
        .context("the previous memory store is absent; this client cannot initialize it")?;
    ensure!(
        identity.instance == expected && identity.initialized,
        "memory store identity changed; this client cannot open a replacement"
    );
    Ok(())
}

fn validate_identity(identity: &Identity) -> Result<()> {
    ensure!(identity.version == 1, "memory project identity mismatch");
    Uuid::parse_str(&identity.instance).context("invalid memory instance identity")?;
    for secret in [&identity.password, &identity.reader_password] {
        ensure!(
            valid_secret(secret),
            "invalid private memory credential record"
        );
    }
    ensure!(
        identity.template.as_deref().is_none_or(valid_template_key),
        "invalid memory template identity"
    );
    Ok(())
}

/// A template key is one portable, case-distinct path component: `[a-z0-9_-]`,
/// at most [`TEMPLATE_KEY_LIMIT`] bytes. The template cache names its key
/// lock, template and build directories after it, so no separator, dot,
/// uppercase letter or other byte a filesystem could reinterpret is accepted.
pub(crate) fn valid_template_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= TEMPLATE_KEY_LIMIT
        && key.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

/// The store template key a staging directory's identity record names, if
/// any, read through the same checked reader and record type as every other
/// identity read. `None` when the stage has no identity record or its record
/// names no template; an unreadable or invalid record is an error.
pub(crate) fn stage_template_key(directory: &Path) -> Result<Option<String>> {
    let Some(identity) = read_record::<Identity>(&directory.join("identity.json"))? else {
        return Ok(None);
    };
    validate_identity(&identity)?;
    Ok(identity.template)
}

/// Publish the identity record of a stage whose `data/` was just copied from
/// the store template `template`: a new instance, new secrets, the project's
/// own scope and `initialized: false`, so the stage's first engine start
/// adopts the copy. It must be the last file written into the stage.
pub(crate) fn write_template_stage_identity(
    directory: &Path,
    project_scope: &str,
    template: &str,
) -> Result<()> {
    let identity = Identity {
        version: 1,
        instance: Uuid::new_v4().to_string(),
        project_scope: project_scope.to_owned(),
        password: secret(),
        reader_password: secret(),
        initialized: false,
        template: Some(template.to_owned()),
    };
    validate_identity(&identity)?;
    ensure!(
        read_record::<Identity>(&directory.join("identity.json"))?.is_none(),
        "memory template stage already has an identity"
    );
    write_record(&directory.join("identity.json"), &identity)
}

/// Publish the identity record of a new store template build: the
/// placeholder instance and scope, new secrets and the template key, so its
/// first engine start writes the placeholder identity row instead of a
/// project's. Opened with [`TEMPLATE_SCOPE`] as its project scope.
pub(crate) fn write_template_build_identity(directory: &Path, template: &str) -> Result<()> {
    let identity = Identity {
        version: 1,
        instance: TEMPLATE_INSTANCE.to_owned(),
        project_scope: TEMPLATE_SCOPE.to_owned(),
        password: secret(),
        reader_password: secret(),
        initialized: false,
        template: Some(template.to_owned()),
    };
    validate_identity(&identity)?;
    ensure!(
        read_record::<Identity>(&directory.join("identity.json"))?.is_none(),
        "memory template build already has an identity"
    );
    write_record(&directory.join("identity.json"), &identity)
}

/// The root and reader secrets of a store template build, for the byte scan
/// that refuses to publish a captured `data/` holding either. Only a build
/// identity (the template instance and scope, uninitialized or initialized
/// by its own build) is read; any other record is refused.
pub(crate) fn template_build_secrets(directory: &Path) -> Result<[String; 2]> {
    let identity = read_record::<Identity>(&directory.join("identity.json"))?
        .context("memory template build identity is missing")?;
    validate_identity(&identity)?;
    ensure!(
        identity.instance == TEMPLATE_INSTANCE
            && identity.project_scope == TEMPLATE_SCOPE
            && identity.template.is_some(),
        "memory store is not a template build"
    );
    Ok([identity.password, identity.reader_password])
}

/// A test's view of one identity record, without its secrets' values.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IdentityView {
    pub(crate) instance: String,
    pub(crate) project_scope: String,
    pub(crate) initialized: bool,
    pub(crate) template: Option<String>,
    /// The root and reader secrets, for credential tests only.
    pub(crate) secrets: (String, String),
}

#[cfg(test)]
impl From<&Identity> for IdentityView {
    fn from(identity: &Identity) -> Self {
        Self {
            instance: identity.instance.clone(),
            project_scope: identity.project_scope.clone(),
            initialized: identity.initialized,
            template: identity.template.clone(),
            secrets: (identity.password.clone(), identity.reader_password.clone()),
        }
    }
}

/// Read one store directory's identity record in a test.
#[cfg(test)]
pub(crate) fn read_identity_view(directory: &Path) -> Result<IdentityView> {
    let identity = read_record::<Identity>(&directory.join("identity.json"))?
        .context("memory identity record is missing")?;
    validate_identity(&identity)?;
    Ok(IdentityView::from(&identity))
}

/// Rewrite one identity record's instance, scope, initialization and
/// template in a test, keeping its secrets.
#[cfg(test)]
pub(crate) fn edit_identity(directory: &Path, edit: impl FnOnce(&mut IdentityView)) -> Result<()> {
    let mut identity = read_record::<Identity>(&directory.join("identity.json"))?
        .context("memory identity record is missing")?;
    let mut view = IdentityView::from(&identity);
    edit(&mut view);
    identity.instance = view.instance;
    identity.project_scope = view.project_scope;
    identity.initialized = view.initialized;
    identity.template = view.template;
    validate_identity(&identity)?;
    write_record(&directory.join("identity.json"), &identity)
}

/// Publish an initialized identity record with new secrets in a test, for a
/// copied `data/` whose `main` identity row holds `instance` and `scope`:
/// the next start authenticates root from the new secret (no `config/` is
/// copied) and changes no row.
#[cfg(test)]
pub(crate) fn write_initialized_identity(
    directory: &Path,
    instance: &str,
    project_scope: &str,
) -> Result<()> {
    let identity = Identity {
        version: 1,
        instance: instance.to_owned(),
        project_scope: project_scope.to_owned(),
        password: secret(),
        reader_password: secret(),
        initialized: true,
        template: None,
    };
    validate_identity(&identity)?;
    write_record(&directory.join("identity.json"), &identity)
}

fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn valid_secret(secret: &str) -> bool {
    secret.len() == 64 && secret.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// One pool's connection progress for its whole life: the latest
/// authentication phase, the last callback rejection and how many new
/// connections entered Kuru's identity callback (after their TCP connect and
/// MySQL authentication). Retained with the pool.
#[derive(Clone)]
pub(crate) struct ConnectionObservation(Arc<ObservationShared>);

struct ObservationShared {
    progress: StdMutex<ConnectionProgress>,
    authenticated: AtomicU64,
    /// Acquisitions through the pool funnel still waiting.
    pending_acquires: tokio::sync::watch::Sender<u64>,
    /// The first authored identity rejection any callback of this pool
    /// returned. It is sticky for the pool's life: the same endpoint cannot
    /// answer differently.
    identity_rejection: tokio::sync::watch::Sender<Option<&'static str>>,
    #[cfg(test)]
    slow_acquire_records: tokio::sync::watch::Sender<u64>,
    /// Each slow-acquire record's window (what its statement budget had left
    /// when the acquisition began), or `None` when the pool ceiling bounded
    /// it, in record order.
    #[cfg(test)]
    slow_acquire_windows: StdMutex<Vec<Option<Duration>>>,
    #[cfg(test)]
    first_release_cut: AtomicBool,
    #[cfg(test)]
    abandoned: AtomicU64,
    #[cfg(test)]
    gate: StdMutex<Option<Arc<GateShared>>>,
    #[cfg(test)]
    release_gate: StdMutex<Option<Arc<GateShared>>>,
}

struct ConnectionProgress {
    phase: &'static str,
    last_failure: Option<(&'static str, &'static str)>,
}

impl ConnectionObservation {
    fn new() -> Self {
        Self(Arc::new(ObservationShared {
            progress: StdMutex::new(ConnectionProgress {
                phase: "after_connect not entered",
                last_failure: None,
            }),
            authenticated: AtomicU64::new(0),
            pending_acquires: tokio::sync::watch::Sender::new(0),
            identity_rejection: tokio::sync::watch::Sender::new(None),
            #[cfg(test)]
            slow_acquire_records: tokio::sync::watch::Sender::new(0),
            #[cfg(test)]
            slow_acquire_windows: StdMutex::new(Vec::new()),
            #[cfg(test)]
            first_release_cut: AtomicBool::new(false),
            #[cfg(test)]
            abandoned: AtomicU64::new(0),
            #[cfg(test)]
            gate: StdMutex::new(None),
            #[cfg(test)]
            release_gate: StdMutex::new(None),
        }))
    }

    fn phase(&self, phase: &'static str) {
        if let Ok(mut progress) = self.0.progress.lock() {
            progress.phase = phase;
        }
    }

    /// An observation for a pool without the identity callback.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Self::new()
    }

    /// The latest authentication phase any connection of this pool reached.
    pub(crate) fn latest_phase(&self) -> &'static str {
        self.0
            .progress
            .lock()
            .map_or("connection observation unavailable", |progress| {
                progress.phase
            })
    }

    /// New connections that entered this pool's authentication callback.
    pub(crate) fn authenticated(&self) -> u64 {
        self.0.authenticated.load(Ordering::SeqCst)
    }

    /// An acquisition through the pool funnel began waiting.
    pub(crate) fn acquire_started(&self) {
        self.0.pending_acquires.send_modify(|pending| *pending += 1);
    }

    /// An acquisition through the pool funnel completed, failed or was
    /// cancelled.
    pub(crate) fn acquire_ended(&self) {
        self.0
            .pending_acquires
            .send_modify(|pending| *pending = pending.saturating_sub(1));
    }

    /// Acquisitions through the pool funnel still waiting.
    pub(crate) fn pending_acquires(&self) -> u64 {
        *self.0.pending_acquires.borrow()
    }

    /// Completes once at least `pending` acquisitions through the pool funnel
    /// are waiting.
    #[cfg(test)]
    pub(crate) async fn pending_acquires_reach(&self, pending: u64) {
        let mut receiver = self.0.pending_acquires.subscribe();
        let _ = receiver.wait_for(|count| *count >= pending).await;
    }

    /// One pending acquisition left its slow-acquire record.
    pub(crate) fn slow_acquire_recorded(&self) {
        #[cfg(test)]
        self.0
            .slow_acquire_records
            .send_modify(|records| *records += 1);
    }

    /// Keep the window of the slow-acquire record about to be counted
    /// (`None` for the pool ceiling).
    #[cfg(test)]
    pub(crate) fn slow_acquire_window(&self, window: Option<Duration>) {
        if let Ok(mut windows) = self.0.slow_acquire_windows.lock() {
            windows.push(window);
        }
    }

    /// The windows of this pool's slow-acquire records, in order.
    #[cfg(test)]
    pub(crate) fn slow_acquire_windows(&self) -> Vec<Option<Duration>> {
        self.0
            .slow_acquire_windows
            .lock()
            .map(|windows| windows.clone())
            .unwrap_or_default()
    }

    /// Slow-acquire records ("still waiting") this pool's acquisitions left.
    #[cfg(test)]
    pub(crate) fn slow_acquire_records(&self) -> u64 {
        *self.0.slow_acquire_records.borrow()
    }

    /// Completes once this pool's acquisitions have left at least `records`
    /// slow-acquire records.
    #[cfg(test)]
    pub(crate) async fn slow_acquire_records_reach(&self, records: u64) {
        let mut receiver = self.0.slow_acquire_records.subscribe();
        let _ = receiver.wait_for(|count| *count >= records).await;
    }

    /// Whether this pool's first connection release was cut at its pool
    /// attempt's deadline, so that connection was closed and identity
    /// verification authenticated its own: one more authenticated connection
    /// that is the bounded close, not churn.
    #[cfg(test)]
    pub(crate) fn first_release_cut(&self) -> bool {
        self.0.first_release_cut.load(Ordering::SeqCst)
    }

    /// Identity callbacks of this pool whose connection never reached the
    /// pool: the callback was cancelled with its timed-out acquire (such as
    /// an opening-phase first acquire that is then retried) or returned an
    /// error, after which SQLx closes the connection and may connect again.
    /// Each is one more authenticated connection that is an abandoned
    /// attempt, not churn of a working session.
    #[cfg(test)]
    pub(crate) fn abandoned_authentications(&self) -> u64 {
        self.0.abandoned.load(Ordering::SeqCst)
    }

    /// Count a new connection entering Kuru's identity callback, its first
    /// line: a TCP or MySQL handshake that never finishes is never counted.
    /// Under an armed test gate, the connection then waits until the gate is
    /// dropped. The returned attempt counts as abandoned unless the callback
    /// completes it, including when it is cancelled while held at the gate.
    async fn authentication_entered(&self) -> CallbackAttempt {
        self.0.authenticated.fetch_add(1, Ordering::SeqCst);
        #[cfg(test)]
        {
            let attempt = CallbackAttempt {
                observation: Some(self.clone()),
            };
            let gate = self
                .0
                .gate
                .lock()
                .expect("authentication gate lock")
                .clone();
            if let Some(gate) = gate {
                self.phase(AUTHENTICATION_GATE_PHASE);
                gate.hold().await;
            }
            attempt
        }
        #[cfg(not(test))]
        CallbackAttempt {}
    }

    /// A checked-out connection is being returned to this pool. Under an
    /// armed test release gate, the return waits until the gate is dropped.
    #[cfg(test)]
    async fn release_entered(&self) {
        let gate = {
            let mut slot = self.0.release_gate.lock().expect("release gate lock");
            if slot.as_ref().is_some_and(|gate| gate.once) {
                slot.take()
            } else {
                slot.clone()
            }
        };
        if let Some(gate) = gate {
            gate.hold().await;
        }
    }

    /// Hold every later new connection of this pool at the start of its
    /// authentication until the returned gate is dropped. No time elapses
    /// inside the gate; tests observe [`ConnectionGate::entered`].
    #[cfg(test)]
    pub(crate) fn gate_new_authentications(&self) -> ConnectionGate {
        let shared = Arc::new(GateShared {
            entered: tokio::sync::watch::Sender::new(false),
            released: tokio::sync::watch::Sender::new(false),
            once: false,
        });
        let previous = self
            .0
            .gate
            .lock()
            .expect("authentication gate lock")
            .replace(shared.clone());
        assert!(previous.is_none(), "authentication gate already armed");
        ConnectionGate {
            shared,
            observation: self.clone(),
            slot: |shared| &shared.gate,
        }
    }

    /// Hold every later return of a checked-out connection to this pool,
    /// before SQLx's release ping, until the returned gate is dropped. A
    /// held return stays pending, so a test can cancel a release mid-flight
    /// without guessing how long the ping takes.
    #[cfg(test)]
    pub(crate) fn gate_releases(&self) -> ConnectionGate {
        self.arm_release_gate(false)
    }

    /// Hold only the next return of a checked-out connection, as
    /// [`Self::gate_releases`] does; later returns pass.
    #[cfg(test)]
    pub(crate) fn gate_next_release(&self) -> ConnectionGate {
        self.arm_release_gate(true)
    }

    #[cfg(test)]
    fn arm_release_gate(&self, once: bool) -> ConnectionGate {
        let shared = Arc::new(GateShared {
            entered: tokio::sync::watch::Sender::new(false),
            released: tokio::sync::watch::Sender::new(false),
            once,
        });
        let previous = self
            .0
            .release_gate
            .lock()
            .expect("release gate lock")
            .replace(shared.clone());
        assert!(previous.is_none(), "release gate already armed");
        ConnectionGate {
            shared,
            observation: self.clone(),
            slot: |shared| &shared.release_gate,
        }
    }

    /// The authored identity rejection this pool's callbacks returned first,
    /// once one has: every later acquisition on the pool fails with it.
    pub(crate) fn identity_rejection(&self) -> Option<sqlx::Error> {
        self.0
            .identity_rejection
            .borrow()
            .map(|cause| sqlx::Error::Protocol(cause.into()))
    }

    /// Completes with the pool's authored identity rejection once any of its
    /// callbacks has returned one, at once if one already has.
    pub(crate) async fn identity_rejected(&self) -> sqlx::Error {
        let mut rejection = self.0.identity_rejection.subscribe();
        let cause = match rejection.wait_for(Option::is_some).await {
            Ok(cause) => (*cause).unwrap_or(IDENTITY_MISMATCH),
            // The sender lives as long as this observation.
            Err(_) => IDENTITY_MISMATCH,
        };
        sqlx::Error::Protocol(cause.into())
    }

    fn rejected(&self, error: &sqlx::Error) {
        if let Some(cause) = identity_rejection(error) {
            self.0.identity_rejection.send_if_modified(|first| {
                let unset = first.is_none();
                first.get_or_insert(cause);
                unset
            });
        }
        // SQLx discards callback errors while retrying acquisition. Keep only
        // authored messages or static error classes, never SQL payloads or
        // connection options. A later attempt may be in a different phase.
        let Ok(mut progress) = self.0.progress.lock() else {
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

    pub(crate) fn diagnostic(&self) -> String {
        let Ok(progress) = self.0.progress.lock() else {
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
        let Ok(progress) = self.0.progress.lock() else {
            return false;
        };
        progress.phase == "after_connect not entered"
            && matches!(error, sqlx::Error::Io(error) if error.kind() == std::io::ErrorKind::ConnectionReset)
    }
}

/// One identity callback in flight. Under test, dropping it before
/// [`Self::accepted`] records the callback as abandoned on its pool's
/// observation; outside tests it carries nothing.
struct CallbackAttempt {
    #[cfg(test)]
    observation: Option<ConnectionObservation>,
}

impl CallbackAttempt {
    /// The callback accepted its connection, which SQLx now hands to the pool.
    fn accepted(self) {
        #[cfg(test)]
        {
            let mut attempt = self;
            attempt.observation = None;
        }
    }
}

#[cfg(test)]
impl Drop for CallbackAttempt {
    fn drop(&mut self) {
        if let Some(observation) = self.observation.take() {
            observation.0.abandoned.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// The phase a gated new connection reports while held.
#[cfg(test)]
pub(crate) const AUTHENTICATION_GATE_PHASE: &str = "authentication gate entered";

#[cfg(test)]
struct GateShared {
    entered: tokio::sync::watch::Sender<bool>,
    released: tokio::sync::watch::Sender<bool>,
    /// Hold only the first connection that reaches the gate.
    once: bool,
}

#[cfg(test)]
impl GateShared {
    /// Report entry, then wait until the gate is dropped.
    async fn hold(&self) {
        self.entered.send_replace(true);
        let mut released = self.released.subscribe();
        let _ = released.wait_for(|released| *released).await;
    }
}

/// A test hold on one pool's new connections (or, from
/// [`ConnectionObservation::gate_releases`], on its connection returns).
/// Dropping it releases every held connection and disarms the pool.
#[cfg(test)]
pub(crate) struct ConnectionGate {
    shared: Arc<GateShared>,
    observation: ConnectionObservation,
    slot: fn(&ObservationShared) -> &StdMutex<Option<Arc<GateShared>>>,
}

#[cfg(test)]
impl ConnectionGate {
    /// Completes once a new connection of the gated pool has entered
    /// authentication.
    pub(crate) async fn entered(&self) {
        let mut entered = self.shared.entered.subscribe();
        let _ = entered.wait_for(|entered| *entered).await;
    }

    pub(crate) fn was_entered(&self) -> bool {
        *self.shared.entered.borrow()
    }
}

#[cfg(test)]
impl Drop for ConnectionGate {
    fn drop(&mut self) {
        // A take-once gate may already have left the slot; never disarm a
        // different gate armed after it.
        if let Ok(mut gate) = (self.slot)(&self.observation.0).lock()
            && gate
                .as_ref()
                .is_some_and(|armed| Arc::ptr_eq(armed, &self.shared))
        {
            gate.take();
        }
        self.shared.released.send_replace(true);
    }
}

struct PoolAttemptOptions {
    /// SQLx's per-acquire timeout for the whole lifetime of the pool.
    acquire_timeout: Duration,
    /// Bound on the pool's first acquisition and its first release. An
    /// acquire that SQLx times out before it ends is retried, which happens
    /// only when it is longer than `acquire_timeout` (an opening startup
    /// budget above `QUERY_TIMEOUT`). `None` for a pool created once memory
    /// is open: the creation's budget scope, whose deadline is taken before
    /// the first acquire and so fires no later than `acquire_timeout`,
    /// bounds both, and nothing is retried.
    first_acquire_window: Option<Duration>,
    /// SQLx retries every `after_connect` error until `acquire_timeout`.
    /// When set, an authored identity rejection ends acquisition instead.
    identity_rejection_is_terminal: bool,
    _test_probe_delay: Option<(Duration, Arc<AtomicBool>)>,
    /// Receives a take-once gate that holds the attempt's first connection
    /// release before SQLx's ping.
    #[cfg(test)]
    _test_first_release_hold: Option<tokio::sync::oneshot::Sender<ConnectionGate>>,
    /// Receives a gate that holds the attempt's new connections at the start
    /// of their authentication.
    #[cfg(test)]
    _test_authentication_gate: Option<tokio::sync::oneshot::Sender<ConnectionGate>>,
}

impl PoolAttemptOptions {
    fn ordinary() -> Self {
        Self {
            acquire_timeout: OPENING_POOL_FLOOR,
            first_acquire_window: Some(OPENING_POOL_FLOOR),
            identity_rejection_is_terminal: false,
            _test_probe_delay: None,
            #[cfg(test)]
            _test_first_release_hold: None,
            #[cfg(test)]
            _test_authentication_gate: None,
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

async fn connect_pool_with_timeout(
    identity: &Identity,
    endpoint: &Endpoint,
    directory: &Path,
    branch: &str,
    read_only: bool,
    max: u32,
    attempt: PoolAttemptOptions,
) -> Result<(MySqlPool, ConnectionObservation)> {
    let (result, observation) = connect_pool_attempt(
        identity, endpoint, directory, branch, read_only, max, attempt,
    )
    .await?;
    match result {
        Ok(pool) => Ok((pool, observation)),
        Err(error) => Err(connection_error(error, &observation)),
    }
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
    #[cfg(test)]
    if let Some(gate) = attempt._test_authentication_gate {
        let _ = gate.send(observation.gate_new_authentications());
    }
    let callback_observation = observation.clone();
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
            #[cfg(test)]
            let test_probe_delay = attempt._test_probe_delay.clone();
            #[cfg(test)]
            let stalled_until = stalled_until.clone();
            Box::pin(async move {
                let attempt = observation.authentication_entered().await;
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
                match &result {
                    Ok(()) => attempt.accepted(),
                    Err(error) => observation.rejected(error),
                }
                result
            })
        });
    #[cfg(test)]
    let pool = {
        let observation = observation.clone();
        pool.after_release(move |_, _| {
            let observation = observation.clone();
            Box::pin(async move {
                observation.release_entered().await;
                Ok(true)
            })
        })
    };
    let pool = pool.connect_lazy_with(options);
    // Equivalent to `connect_with` (one acquire, then release) when the first
    // window equals the lifetime timeout. A longer opening window retries
    // timed-out acquires without changing any later acquire on this pool.
    // Without a first window the caller's budget scope bounds the acquire,
    // and SQLx's lifetime timeout, never earlier than that scope, ends it
    // without a retry.
    let first_deadline = attempt
        .first_acquire_window
        .map(|window| Instant::now() + window);
    let acquired = loop {
        // Boxed: SQLx's acquire, which establishes the lazy connection, is
        // about eighty nested future layers, and this attempt already sits
        // deep inside store and server opens (the select below adds more).
        // Inline, it pushes every opener's layout past the compiler's query
        // depth limit; one allocation per attempt keeps it bounded.
        let acquiring = Box::pin(async {
            match first_deadline {
                Some(deadline) => timeout_at(deadline, pool.acquire()).await,
                None => Ok(pool.acquire().await),
            }
        });
        let acquired = if attempt.identity_rejection_is_terminal {
            tokio::select! {
                biased;
                error = observation.identity_rejected() => Ok(Err(error)),
                acquired = acquiring => acquired,
            }
        } else {
            acquiring.await
        };
        match acquired {
            Ok(Err(sqlx::Error::PoolTimedOut))
                if first_deadline.is_some_and(|deadline| Instant::now() < deadline) => {}
            Ok(acquired) => break acquired,
            Err(_) => break Err(sqlx::Error::PoolTimedOut),
        }
    };
    let result = match acquired {
        Ok(mut connection) => {
            // Return the first connection inline so the next statement on
            // this pool reuses it instead of racing SQLx's spawned release.
            // While opening, the attempt's own deadline bounds the release
            // ping: when it runs out the future is dropped, SQLx closes the
            // connection, and the identity verification opens one under its
            // own deadline. Once open, the creation's budget scope bounds it
            // and its expiry fails the creation with this pool dropped.
            #[cfg(test)]
            if let Some(hold) = attempt._test_first_release_hold {
                let _ = hold.send(observation.gate_next_release());
            }
            let returning = connection.return_to_pool();
            let returned = match first_deadline {
                Some(deadline) => timeout_at(deadline, returning).await.is_ok(),
                None => {
                    returning.await;
                    true
                }
            };
            if !returned {
                #[cfg(test)]
                observation
                    .0
                    .first_release_cut
                    .store(true, Ordering::SeqCst);
                tracing::warn!(
                    branch,
                    "memory pool's first connection release exceeded the attempt's deadline; \
                     the connection was closed instead of returned to the pool"
                );
            }
            Ok(pool)
        }
        Err(error) => Err(error),
    };
    Ok((result, observation))
}

async fn verify_identity<'p, P>(pool: P, directory: &Path, identity: &Identity) -> Result<()>
where
    P: sqlx::Executor<'p, Database = sqlx::MySql> + Copy,
{
    verify_identity_until(
        pool,
        directory,
        identity,
        Instant::now() + OPENING_POOL_FLOOR,
    )
    .await
}

async fn verify_identity_until<'p, P>(
    pool: P,
    directory: &Path,
    identity: &Identity,
    deadline: Instant,
) -> Result<()>
where
    P: sqlx::Executor<'p, Database = sqlx::MySql> + Copy,
{
    crate::pool::within_until(deadline, async {
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
        let response = if TemplateVerdict::find(&error).is_some() {
            Response::TemplateRejected(message)
        } else {
            Response::Failed(message)
        };
        let _ = timeout(KILL_GRACE, write_frame(output, &response)).await;
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
    prepare_directory(
        &request.directory,
        request.read_only || request.expected_instance.is_some(),
    )?;
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
    if let Some(expected) = &request.expected_instance {
        require_existing_instance(&request.directory, &request.project_scope, expected)?;
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
                template: None,
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
        "log_level: warning\nlog_format: text\n{SERVER_BEHAVIOR}listener:\n  host: 127.0.0.1\n  port: {port}\n  max_connections: 32\n  max_connections_timeout_millis: 1000\n  read_timeout_millis: {read_timeout}\n  write_timeout_millis: 5000\n  allow_cleartext_passwords: false\ndata_dir: {}\ncfg_dir: {}\nprivilege_file: {}\nbranch_control_file: {}\nsystem_variables:\n  datadir: {}\n  secure_file_priv: {}\n",
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

/// The server that accepted the authenticated startup probe serves another
/// data directory. Nothing was written to it. Template copies share their
/// credentials, so a sibling's Dolt that bound this store's freshly selected
/// port first authenticates the probe; the owned Dolt is then still starting
/// and will report the taken port itself.
#[derive(Debug)]
struct ForeignDataDirectory;

impl std::fmt::Display for ForeignDataDirectory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Dolt bootstrap data directory mismatch")
    }
}

impl std::error::Error for ForeignDataDirectory {}

async fn start_database(
    child: &mut Child,
    directory: &Path,
    identity: &mut Identity,
    endpoint: &Endpoint,
    deadline: Instant,
) -> Result<()> {
    // A foreign server that answered the probe: retained only for the error.
    let mut foreign: Option<anyhow::Error> = None;
    loop {
        if let Some(status) = child.try_wait()? {
            let exit = anyhow::Error::new(DoltPrematureExit(status));
            return Err(match foreign {
                Some(_) => exit.context(format!(
                    "{ForeignDataDirectory}: another server answered on port {}",
                    endpoint.port
                )),
                None => exit,
            });
        }
        if Instant::now() >= deadline {
            let exceeded = "authenticated Dolt startup deadline exceeded";
            return Err(match foreign {
                Some(error) => error.context(exceeded),
                None => anyhow!(exceeded),
            });
        }
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
            #[cfg(all(test, unix))]
            adoption_fault::engine_started(child);
            let initialized =
                bootstrap_database(&pool, directory, identity, deadline, &mut foreign).await;
            pool.close().await;
            match initialized {
                // Not this store's server: keep observing the owned Dolt,
                // whose own exit reports the taken port to the bounded retry.
                Ok(BootstrapOutcome::Foreign(error)) => {
                    foreign = Some(error);
                }
                Ok(BootstrapOutcome::Initialized) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        sleep(Duration::from_millis(25)).await;
    }
}

#[derive(Debug)]
enum BootstrapOutcome {
    Initialized,
    Foreign(anyhow::Error),
}

async fn bootstrap_database(
    pool: &MySqlPool,
    directory: &Path,
    identity: &mut Identity,
    deadline: Instant,
    foreign: &mut Option<anyhow::Error>,
) -> Result<BootstrapOutcome> {
    let mut phase = "checking the bootstrap data directory";
    let initialized = timeout(deadline.saturating_duration_since(Instant::now()), async {
        #[cfg(all(test, unix))]
        bootstrap_fault::before_datadir(foreign.is_some()).await;
        initialize_database(pool, directory, identity, &mut phase, foreign).await
    })
    .await;
    match initialized {
        Ok(Err(error)) if error.downcast_ref::<ForeignDataDirectory>().is_some() => {
            Ok(BootstrapOutcome::Foreign(error))
        }
        Err(_) if foreign.is_some() => Err(foreign.take().expect("retained foreign observation")
            .context("the last data-directory probe timed out; retaining the previously observed foreign listener")
            .context("authenticated Dolt startup deadline exceeded")),
        initialized => {
            initialized
                .with_context(|| format!("Dolt database bootstrap deadline exceeded while {phase}"))?
                .with_context(|| format!("Dolt database bootstrap failed while {phase}"))?;
            Ok(BootstrapOutcome::Initialized)
        }
    }
}

async fn initialize_database(
    pool: &MySqlPool,
    directory: &Path,
    identity: &mut Identity,
    phase: &mut &'static str,
    foreign: &mut Option<anyhow::Error>,
) -> Result<()> {
    let datadir: String = sqlx::query_scalar("SELECT @@datadir")
        .fetch_one(pool)
        .await?;
    ensure!(
        same_directory(Path::new(&datadir), &directory.join("data"))
            .with_context(|| format!("resolve Dolt bootstrap datadir {datadir:?}"))?,
        ForeignDataDirectory
    );
    // This probe reached the correct server: earlier foreign evidence must
    // not describe a later failure in this server's bootstrap.
    *foreign = None;
    // A pending template identity names the key it was created under. Only
    // while the store is uninitialized is that key compared, before any
    // write: once adopted, the key is provenance, and a later build opens the
    // store whatever key it was compiled with.
    let adopting = match (&identity.template, identity.initialized) {
        (Some(key), false) => {
            *phase = "comparing the store template key";
            let compiled = compiled_template_key();
            ensure!(
                key == compiled,
                "memory store template key {key:?} differs from this supervisor's compiled \
                 template key {compiled:?}; its data was not written"
            );
            if identity.instance == TEMPLATE_INSTANCE {
                // A template build: today's bootstrap writes the placeholder.
                ensure!(
                    identity.project_scope == TEMPLATE_SCOPE,
                    "a memory template build identity must use the template scope"
                );
                false
            } else {
                true
            }
        }
        _ => false,
    };
    if adopting {
        adopt_template(pool, identity, phase).await?;
    } else if !identity.initialized {
        *phase = "creating the project database";
        #[cfg(all(test, unix))]
        bootstrap_fault::before_project_create().await;
        sqlx::query(BOOTSTRAP_CREATE_DATABASE).execute(pool).await?;
        *phase = "creating the project identity table";
        sqlx::query(BOOTSTRAP_CREATE_IDENTITY).execute(pool).await?;
        *phase = "initializing the project identity";
        let existing: i64 = sqlx::query_scalar(BOOTSTRAP_COUNT_IDENTITY)
            .fetch_one(pool)
            .await?;
        if existing == 0 {
            sqlx::query(BOOTSTRAP_INSERT_IDENTITY)
                .bind(&identity.instance)
                .bind(&identity.project_scope)
                .execute(pool)
                .await?;
        }
    }
    if !identity.initialized {
        // Dolt's CREATE USER parser rejects bind parameters. Only our generated
        // 64-character hex credential can enter this literal; never user input.
        ensure!(
            valid_secret(&identity.reader_password),
            "invalid private reader credential"
        );
        let [before, after] = BOOTSTRAP_READER_ACCOUNT;
        let credential_sql = format!("{before}{}{after}", identity.reader_password);
        *phase = "configuring the private reader account";
        sqlx::query(sqlx::AssertSqlSafe(credential_sql))
            .execute(pool)
            .await
            .map_err(|_| anyhow!("configuring private read-only memory account failed"))?;
        *phase = "granting private reader access";
        sqlx::query(BOOTSTRAP_READER_GRANT).execute(pool).await?;
    }
    if adopting {
        // Both adopted rows, before the identity is marked initialized.
        for (query, reference) in [
            (
                "SELECT instance_id, project_scope FROM `kuru/kuru_usage_v1`.kuru_instance WHERE singleton = 1",
                "usage branch",
            ),
            (
                "SELECT instance_id, project_scope FROM kuru.kuru_instance WHERE singleton = 1",
                "main",
            ),
        ] {
            *phase = "verifying the adopted project identity";
            let rows = sqlx::query(query).fetch_all(pool).await?;
            if !identity_rows_are(&rows, &identity.instance, &identity.project_scope)? {
                return Err(TemplateVerdict::new(format!(
                    "the adopted {reference} identity row differs from the new project \
                     identity ({} rows)",
                    rows.len()
                ))
                .into());
            }
        }
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

#[cfg(all(test, unix))]
mod bootstrap_fault {
    use super::*;

    #[derive(Clone)]
    pub(super) enum Fault {
        RepeatedForeignProbe(Arc<std::sync::atomic::AtomicBool>),
        ProjectCreate(Arc<std::sync::atomic::AtomicBool>),
    }

    tokio::task_local! {
        pub(super) static FAULT: Fault;
    }

    pub(super) async fn before_datadir(foreign_observed: bool) {
        if foreign_observed
            && let Ok(Fault::RepeatedForeignProbe(entered)) = FAULT.try_with(Clone::clone)
        {
            entered.store(true, std::sync::atomic::Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
    }

    pub(super) async fn before_project_create() {
        if let Ok(Fault::ProjectCreate(entered)) = FAULT.try_with(Clone::clone) {
            entered.store(true, std::sync::atomic::Ordering::SeqCst);
            // The real datadir query has finished. Only this test runtime's
            // injected stall now advances to the existing deadline virtually.
            tokio::time::pause();
            std::future::pending::<()>().await;
        }
    }
}

/// Whether `rows` is exactly one identity row holding `instance` and `scope`.
fn identity_rows_are(rows: &[sqlx::mysql::MySqlRow], instance: &str, scope: &str) -> Result<bool> {
    let [row] = rows else {
        return Ok(false);
    };
    Ok(row.try_get::<String, _>("instance_id")? == instance
        && row.try_get::<String, _>("project_scope")? == scope)
}

/// Give a copied template stage its own identity, once, on one session.
///
/// First, on the usage branch and on `main`, the working set must be clean
/// and the compiled placeholder must be the only identity row. Only then is
/// either ref rewritten: the usage branch first, so `main` is never adopted
/// while the usage branch still holds the placeholder, then `main`, each by
/// one guarded rewrite that must change exactly one row and one Dolt commit.
/// A completed comparison with another result is a [`TemplateVerdict`];
/// every SQL, transport or engine failure is an ordinary error. Nothing
/// reconciles a partial adoption: recovery preserves the stage without
/// starting an engine on it.
async fn adopt_template(
    pool: &MySqlPool,
    identity: &Identity,
    phase: &mut &'static str,
) -> Result<()> {
    *phase = "opening the template adoption session";
    // The bootstrap pool holds one connection: detach it, so pool queries
    // after adoption open their own session instead of waiting for this one.
    let mut connection = pool.acquire().await?.detach();
    let adopted = adopt_on(&mut connection, identity, phase).await;
    let closed = sqlx::Connection::close(connection)
        .await
        .context("close the template adoption session");
    match (adopted, closed) {
        (Ok(()), closed) => closed,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(close)) => Err(error.context(format!(
            "template adoption session close also failed: {close:#}"
        ))),
    }
}

/// The two adopted refs, in adoption order.
const ADOPTED_REFS: [(&str, &str); 2] = [(USAGE_DATABASE, "usage branch"), ("kuru", "main")];

async fn adopt_on(
    connection: &mut sqlx::MySqlConnection,
    identity: &Identity,
    phase: &mut &'static str,
) -> Result<()> {
    use sqlx::Executor;
    for (database, reference) in ADOPTED_REFS {
        *phase = if database == USAGE_DATABASE {
            "verifying the store template placeholder on the usage branch"
        } else {
            "verifying the store template placeholder on main"
        };
        // Both names are constants of this module; `USE` takes no parameter.
        connection
            .execute(sqlx::AssertSqlSafe(format!("USE `{database}`")))
            .await?;
        let changes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status")
            .fetch_one(&mut *connection)
            .await?;
        if changes != 0 {
            return Err(TemplateVerdict::new(format!(
                "the {reference} working set holds {changes} uncommitted changes"
            ))
            .into());
        }
        let rows =
            sqlx::query("SELECT instance_id, project_scope FROM kuru_instance WHERE singleton = 1")
                .fetch_all(&mut *connection)
                .await?;
        if !identity_rows_are(&rows, TEMPLATE_INSTANCE, TEMPLATE_SCOPE)? {
            return Err(TemplateVerdict::new(format!(
                "the {reference} identity row is not the compiled template placeholder ({} rows)",
                rows.len()
            ))
            .into());
        }
    }
    for (database, reference) in ADOPTED_REFS {
        let usage = database == USAGE_DATABASE;
        *phase = if usage {
            "adopting the store template on the usage branch"
        } else {
            "adopting the store template on main"
        };
        connection
            .execute(sqlx::AssertSqlSafe(format!("USE `{database}`")))
            .await?;
        #[cfg(all(test, unix))]
        let guard_scope = adoption_fault::guard_scope(usage);
        #[cfg(not(all(test, unix)))]
        let guard_scope = TEMPLATE_SCOPE;
        let updated = sqlx::query(
            "UPDATE kuru_instance SET instance_id = ?, project_scope = ? WHERE singleton = 1 AND instance_id = ? AND project_scope = ?",
        )
        .bind(&identity.instance)
        .bind(&identity.project_scope)
        .bind(TEMPLATE_INSTANCE)
        .bind(guard_scope)
        .execute(&mut *connection)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(TemplateVerdict::new(format!(
                "the guarded {reference} identity rewrite changed {updated} rows instead of one"
            ))
            .into());
        }
        sqlx::query("CALL DOLT_COMMIT('-am', ?, '--author', ?)")
            .bind(ADOPTION_MESSAGE)
            .bind(crate::store::AUTHOR)
            .fetch_all(&mut *connection)
            .await?;
        #[cfg(all(test, unix))]
        adoption_fault::reach(if usage {
            adoption_fault::Point::AfterUsageCommit
        } else {
            adoption_fault::Point::AfterMainCommit
        })
        .await?;
    }
    Ok(())
}

/// Test-only faults inside the adoption group of an in-process supervisor.
/// A test scopes one with [`adoption_fault::FAULT`] around `supervise`; the
/// bootstrap runs in that task. Ordinary supervisors run without one.
#[cfg(all(test, unix))]
pub(crate) mod adoption_fault {
    use super::*;

    /// A point in the adoption group, after the named ref's commit.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum Point {
        AfterUsageCommit,
        AfterMainCommit,
    }

    #[derive(Clone, Debug)]
    pub(crate) enum Fault {
        /// No fault: the bootstrap runs as it does in a spawned supervisor.
        Nothing,
        /// Fail the bootstrap at the point: the stage is left as a crash
        /// there would leave it.
        FailAt(Point),
        /// Wait at the point until the bootstrap deadline ends the start.
        StallAt(Point),
        /// Kill the owned Dolt child at the point. The supervisor has not
        /// reaped it, so its process ID still names it.
        KillEngineAt(Point, Arc<StdMutex<Option<u32>>>),
        /// Guard the usage branch rewrite with a scope no row holds, so the
        /// engine reports zero changed rows.
        MissUsageRewrite,
    }

    tokio::task_local! {
        pub(crate) static FAULT: Fault;
    }

    fn current() -> Option<Fault> {
        FAULT.try_with(Clone::clone).ok()
    }

    pub(super) fn guard_scope(usage: bool) -> &'static str {
        if usage && matches!(current(), Some(Fault::MissUsageRewrite)) {
            "project/0000000000000000000000000000000000000000000000000000000000000000"
        } else {
            TEMPLATE_SCOPE
        }
    }

    /// Record the owned engine's process ID for [`Fault::KillEngineAt`].
    pub(super) fn engine_started(child: &Child) {
        if let Some(Fault::KillEngineAt(_, engine)) = current() {
            *engine.lock().expect("engine slot") = child.id();
        }
    }

    /// How an in-process supervisor's startup ended.
    #[derive(Debug)]
    pub(crate) enum Outcome {
        Ready,
        Failed(String),
        TemplateRejected(String),
    }

    /// Run one supervisor for `options` in this process with `fault` scoped
    /// around its bootstrap, read its startup response, then close its
    /// lifetime and wait, bounded, for it to reap Dolt and release the stage's
    /// lifecycle lease. The response is exactly the frame an owning client
    /// would read. Nothing here registers in the engine ledger: the caller
    /// records the stage's quiescence itself.
    pub(crate) async fn supervise_once(options: &ServerOptions, fault: Fault) -> Result<Outcome> {
        let request = Request {
            expected_instance: None,
            binary: options.binary.clone(),
            directory: fs::canonicalize(&options.directory)?,
            project_scope: options.project_scope.clone(),
            timeout_millis: options.timeout.as_millis().try_into()?,
            read_only: options.read_only,
            lifecycle_root: options.lifecycle_root.clone(),
        };
        let bound = options.timeout + SUPERVISOR_REAP_ALLOWANCE + SUPERVISOR_TRANSPORT_ALLOWANCE;
        let (parent, mut input) = tokio::io::duplex(1024);
        let (mut output, mut response) = tokio::io::duplex(64 * 1024);
        let mut supervisor = tokio::spawn(async move {
            // Held across the real Dolt spawn; see `crate::spawn_gate`.
            let _gate = crate::spawn_gate::spawning().await;
            FAULT
                .scope(fault, supervisor_request(request, &mut input, &mut output))
                .await
        });
        let read = timeout(bound, read_frame::<_, Response>(&mut response)).await;
        // Closing the lifetime stops a ready engine; a failed start has
        // already stopped and reaped its engine before it answered.
        drop(parent);
        let finished = timeout(bound, &mut supervisor).await;
        if finished.is_err() {
            supervisor.abort();
            let _ = supervisor.await;
            bail!("in-process memory supervisor did not finish within {bound:?}");
        }
        let outcome = match read.context("in-process supervisor response deadline exceeded")?? {
            Response::Ready { .. } => Outcome::Ready,
            Response::Failed(message) => Outcome::Failed(message),
            Response::TemplateRejected(message) => Outcome::TemplateRejected(message),
        };
        Ok(outcome)
    }

    pub(super) async fn reach(point: Point) -> Result<()> {
        match current() {
            Some(Fault::FailAt(at)) if at == point => {
                bail!("injected template adoption failure at {point:?}")
            }
            Some(Fault::StallAt(at)) if at == point => std::future::pending().await,
            Some(Fault::KillEngineAt(at, engine)) if at == point => {
                let pid = engine
                    .lock()
                    .expect("engine slot")
                    .context("the owned engine's process ID was not recorded")?;
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(i32::try_from(pid)?),
                    nix::sys::signal::Signal::SIGKILL,
                )?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
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
            template: None,
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
#[cfg(test)]
#[path = "server/template_identity_tests.rs"]
mod template_identity_tests;
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
