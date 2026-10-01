//! Isolated real-engine fixtures shared by workspace behavioral tests.
//!
//! Cargo owns and can replace its profile's top-level executable alias while
//! another package is already testing. Ordinary mise test tasks explicitly use
//! the snapshot prepared before they start. Coverage deliberately does not opt
//! in: its one workspace build supplies the actual instrumented executable.
//!
//! # Temporary stores
//!
//! [`MemoryStore::temporary`] is the default isolated store for tests that
//! only need a current-schema store of their own. It copies a pre-migrated
//! template (see `template.rs` in this module) and then performs the ordinary
//! existing-store open, so each test still owns its private directory,
//! writable open, supervisor and Dolt process. Every copy shares the
//! template's instance identity, credentials, initial revision and migration
//! receipts.
//!
//! [`MemoryStore::temporary_cold`] performs the complete cold open instead:
//! a new identity, initialization, every migration, staged validation and
//! activation. Use it for tests of server, supervisor or process lifecycle,
//! migration, legacy import or activation, and for any test that compares
//! instance identity, credentials or migration receipts, or that needs
//! migrations to run. When in doubt, use the cold constructor.
/// Fixture roots whose teardown refuses to release a live memory owner.
mod fixture_dir;
pub use fixture_dir::{CreatorTeardown, TempDir, release_after_creator_exit};
/// In-process service owners a fixture retires, or reports, on every exit
/// path, including an elapsed fixture deadline.
#[cfg(test)]
mod served_owner;
#[cfg(test)]
pub(crate) use served_owner::{
    FixtureDeadline, ServeEvents, expect_events, expect_no_event, next_event, observed,
    serve_without_deadline,
};
/// Data-tree copy, byte scan and cross-OS capture format for the engine
/// contract tests.
#[cfg(test)]
pub(crate) mod engine_contract;
/// Process-local live-owner and quiescence records the fixture guard reads.
pub(crate) mod engine_ledger;
/// Env-gated lifecycle ordering measurement trace (inert unless enabled).
pub mod lifecycle_trace;
pub(crate) mod template;
#[cfg(windows)]
pub mod windows;
use crate::{MemoryStore, OpenOptions, PublicTurnRecord, SessionCatalogRecord, files};
use anyhow::{Context, Error, Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication, seal_private};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration,
};

/// A one-shot test barrier after a complete typed service request frame and
/// before the call returns its reply. The owner continues independently; the
/// client reads and holds the reply frame until [`ReplyBarrier::release`].
#[derive(Clone, Default)]
pub struct ReplyBarrier {
    pub(crate) inner: Arc<crate::service::rpc::ReplyPause>,
}

impl ReplyBarrier {
    pub async fn wait_sent(&self) {
        self.inner.sent.notified().await;
    }

    /// Wait until the owner's reply frame has arrived and is held. The owner
    /// writes it only after the request's handler returned and its receipt
    /// settled, so a call cancelled after this leaves the owner with no
    /// request in hand. It is not needed for a definite reconcile.
    pub async fn wait_replied(&self) {
        self.inner.replied.notified().await;
    }

    pub fn promotion_sent(&self) -> bool {
        self.inner
            .promotion_sent
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn release(&self) {
        self.inner.release.notify_one();
    }
}

const LIMIT: u64 = 512 * 1024 * 1024;
const DIRECTORY: &str = "kuru-test-supervisors";
const STARTUP_LOG_BYTES: u64 = 40 * 1024;
const STARTUP_TAIL_BYTES: usize = 4 * 1024;
const MAX_STAGE_ENTRIES: usize = 64;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u8,
    sha256: String,
    bytes: u64,
}

impl Receipt {
    fn name(&self) -> Result<String> {
        ensure!(
            self.schema_version == 1
                && self.bytes > 0
                && self.bytes <= LIMIT
                && self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "invalid prepared supervisor receipt"
        );
        Ok(format!("{}{}", self.sha256, std::env::consts::EXE_SUFFIX))
    }
}

/// Private fixture root, including a protected Windows DACL.
pub fn tempdir() -> Result<TempDir> {
    TempDir::new("kuru-fixture-", None)
}

/// Warm the shared test cache once per test process, before any fixture's
/// own deadline: provision the bundled Dolt runtime, then make sure this
/// build's store template is published beside it. Returns the engine.
///
/// The engine step is bounded by the provisioner's cache-lock peer budget
/// plus its version-probe budget and [`WARM_UP_MARGIN`], so the product's own
/// lock error wins that race and an installing caller's extraction and probe
/// are covered; its result, including a failure, is shared by every later
/// caller in the process.
///
/// The template step checks a published template's structure under its
/// shared key lock, or takes the exclusive key lock and builds it, waiting
/// for a peer process's build by polling, all within one fresh-open budget
/// plus [`WARM_UP_MARGIN`] ([`template_warm_up_bound`]). Only its success is
/// cached: a failure fails the fixture whose warm-up met it, named, and the
/// next fixture's warm-up tries again. A lock-file error is fatal here, never
/// "no template". Both steps hold the spawn gate, so neither may start while
/// the caller holds one.
pub async fn warm_runtime_cache() -> Result<PathBuf> {
    let engine = warm_engine().await?;
    // Boxed: a template build nests a complete engine start and the chain.
    Box::pin(warm_template_in(
        &WARMED_TEMPLATE,
        &shared_template_root()?,
        &template_engine(engine.clone())?,
        template_warm_up_bound(),
    ))
    .await?;
    Ok(engine)
}

/// The template half of [`warm_runtime_cache`] in this process: how its one
/// successful warm-up found the shared template.
static WARMED_TEMPLATE: tokio::sync::OnceCell<TemplateWarmUp> = tokio::sync::OnceCell::const_new();

/// How a successful template warm-up found the template.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TemplateWarmUp {
    /// A structurally valid template was already published.
    Published,
    /// This warm-up built and published it.
    Built,
}

/// How this process's template warm-up found the shared template, once it
/// succeeded.
#[cfg(test)]
pub(crate) fn warmed_template() -> Option<TemplateWarmUp> {
    WARMED_TEMPLATE.get().copied()
}

/// The engine half of [`warm_runtime_cache`].
async fn warm_engine() -> Result<PathBuf> {
    static WARMED: tokio::sync::OnceCell<std::result::Result<PathBuf, String>> =
        tokio::sync::OnceCell::const_new();
    WARMED
        .get_or_init(|| async {
            // Provisioning probes the extracted engine with a child process.
            #[cfg(test)]
            let _gate = crate::spawn_gate::spawning().await;
            let config = OpenOptions::new(PathBuf::new(), String::new()).config;
            let cache = crate::store::test_cache();
            let bound = crate::provision::LOCK_TIMEOUT
                .saturating_add(crate::provision::VERSION_TIMEOUT)
                .saturating_add(WARM_UP_MARGIN);
            match tokio::time::timeout(bound, crate::provision::provision(&config, &cache)).await {
                Ok(Ok(binary)) => Ok(binary),
                Ok(Err(error)) => Err(format!("warm test Dolt runtime cache: {error:#}")),
                Err(_) => Err(format!("warm test Dolt runtime cache exceeded {bound:?}")),
            }
        })
        .await
        .clone()
        .map_err(Error::msg)
}

/// Allowance beyond the provisioner's lock and probe budgets for extraction
/// bookkeeping, so its own deadline errors are observed before the warm-up's.
pub const WARM_UP_MARGIN: std::time::Duration = std::time::Duration::from_secs(5);

/// The store template step's bound: one fresh-open budget (whose four
/// engine starts cover one build start, the chain and the capture with
/// room, and a peer process's build) plus [`WARM_UP_MARGIN`].
pub(crate) fn template_warm_up_bound() -> Duration {
    fresh_open_budget().saturating_add(WARM_UP_MARGIN)
}

/// The store templates root of the shared test cache.
pub(crate) fn shared_template_root() -> Result<PathBuf> {
    let cache = crate::store::test_cache();
    let cache = fs::canonicalize(&cache)
        .with_context(|| format!("resolve the shared test cache {}", cache.display()))?;
    Ok(crate::store::creation_template::root_in(&cache))
}

/// Whether `root` is the shared test cache's templates root, which every
/// fixture shares and no fixture's open may build in or quarantine from
/// unless it opted in.
pub(crate) fn is_shared_template_root(root: &Path) -> bool {
    match (shared_template_root(), fs::canonicalize(root)) {
        (Ok(shared), Ok(root)) => shared == root,
        _ => false,
    }
}

/// The engine fixtures build store templates with: the warmed engine, the
/// test supervisor (instrumented under coverage, the prepared snapshot
/// otherwise; the template key excludes it) and the default startup budget.
fn template_engine(binary: PathBuf) -> Result<crate::store::creation_template::Engine> {
    Ok(crate::store::creation_template::Engine {
        binary,
        supervisor: crate::store::test_supervisor()?,
        timeout: default_startup(),
    })
}

/// The template step of [`warm_runtime_cache`], on `cell`: cache only
/// success, bound the whole step, hold the spawn gate.
pub(crate) async fn warm_template_in(
    cell: &tokio::sync::OnceCell<TemplateWarmUp>,
    root: &Path,
    engine: &crate::store::creation_template::Engine,
    bound: Duration,
) -> Result<TemplateWarmUp> {
    cell.get_or_try_init(|| async {
        // Builds start a supervisor and Dolt; see `crate::spawn_gate`.
        #[cfg(test)]
        let _gate = crate::spawn_gate::spawning().await;
        let deadline = std::time::Instant::now() + bound;
        let ensured = tokio::time::timeout(
            bound,
            crate::store::creation_template::ensure_in(
                root,
                engine,
                crate::store::creation_template::Wait::Until(deadline),
            ),
        )
        .await;
        match ensured {
            Ok(Ok(crate::store::creation_template::Ensured::Built(report))) => {
                if report.published {
                    Ok(TemplateWarmUp::Built)
                } else {
                    Err(anyhow::anyhow!(
                        "warm the test store template cache {}: the template was built but could \
                         not be published; its verified stage is left for the next build",
                        root.display()
                    ))
                }
            }
            Ok(Ok(crate::store::creation_template::Ensured::Published)) => {
                Ok(TemplateWarmUp::Published)
            }
            Ok(Ok(crate::store::creation_template::Ensured::Unavailable(unavailable))) => {
                Err(anyhow::anyhow!(
                    "warm the test store template cache {}: unavailable ({unavailable:?})",
                    root.display()
                ))
            }
            Ok(Err(failure)) => Err(anyhow::anyhow!(
                "warm the test store template cache {}: {failure}",
                root.display()
            )),
            Err(_) => Err(anyhow::anyhow!(
                "warm the test store template cache {} exceeded {bound:?}",
                root.display()
            )),
        }
    })
    .await
    .copied()
}

/// Build or verify this build's store template in an engine cache, waiting
/// for a peer's build within [`template_warm_up_bound`]: the `prefetch`
/// task's step after provisioning, with its prepared supervisor snapshot.
pub async fn warm_template_cache(cache: &Path, engine: &Path, supervisor: &Path) -> Result<()> {
    let cache = fs::canonicalize(cache)?;
    let bound = template_warm_up_bound();
    let deadline = std::time::Instant::now() + bound;
    let engine = crate::store::creation_template::Engine {
        binary: engine.to_owned(),
        supervisor: supervisor.to_owned(),
        timeout: default_startup(),
    };
    let root = crate::store::creation_template::root_in(&cache);
    tokio::time::timeout(
        bound,
        Box::pin(crate::store::creation_template::ensure_in(
            &root,
            &engine,
            crate::store::creation_template::Wait::Until(deadline),
        )),
    )
    .await
    .with_context(|| format!("prefetch the store template exceeded {bound:?}"))?
    .map_err(|failure| anyhow::anyhow!("prefetch the store template {}: {failure}", root.display()))
    .and_then(|ensured| match ensured {
        crate::store::creation_template::Ensured::Built(report) if !report.published => {
            Err(anyhow::anyhow!(
                "prefetch the store template {}: built but not published",
                root.display()
            ))
        }
        _ => Ok(()),
    })
}

/// Allowance for creating a fixture child process and its runtime before the
/// product's own startup clock begins inside it.
#[cfg(test)]
pub(crate) const CHILD_START_MARGIN: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(any(test, feature = "test-support"))]
fn default_startup() -> std::time::Duration {
    let config = OpenOptions::new(PathBuf::new(), String::new()).config;
    std::time::Duration::from_secs(config.startup_timeout_secs)
}

/// One owned Dolt server start: the configured startup timeout plus the
/// supervisor-transport allowance `Server::open` adds to it.
pub(crate) fn server_start_budget() -> std::time::Duration {
    default_startup().saturating_add(crate::server::SUPERVISOR_TRANSPORT_ALLOWANCE)
}

/// A fresh store open (`MemoryStore::open_inner` with no active directory):
/// the startup lock wait, four server starts (initialization, migration,
/// validation, active) each with one `QUERY_TIMEOUT` session, three owned
/// server closes, and the staged directory's quiescence wait.
pub(crate) fn fresh_open_budget() -> std::time::Duration {
    let startup = default_startup();
    startup
        .saturating_add(
            server_start_budget()
                .saturating_add(crate::store::QUERY_TIMEOUT)
                .saturating_mul(4),
        )
        .saturating_add(crate::server::close_budget().saturating_mul(3))
        .saturating_add(startup)
}

/// Outer hang backstop for a fixture with `fresh` real lifecycles that create
/// their store and `reopened` real lifecycles that reopen an existing one,
/// under a single-stall model.
///
/// A lifecycle is a `ServiceOwner::open`, a spawned service owner, or a local
/// `MemoryStore::open` that starts Dolt; managed attaches and checked rebinds
/// start none. Every Dolt server start the fixture really performs gets the
/// full `server_start_budget()`: four for a fresh open (initialization,
/// migration, validation, active) and one for a reopen. On top of those, the
/// fixture gets one single-stall term, the largest bound any other single
/// product step can reach (an owned server close, a `QUERY_TIMEOUT` statement,
/// or a startup-budgeted lock, quiescence or maintenance-permit wait), and one
/// `QUERY_TIMEOUT` for its own operations.
///
/// So one stalled step with its own product bound reports that error before
/// this backstop expires; the backstop does not budget several steps each
/// running to its limit. Steps without a product bound, such as warm cache
/// verification, legacy import preparation and activation reads, rely on this
/// backstop alone. Fixtures call [`warm_runtime_cache`] first, so no cold
/// runtime install is charged here, and keep the `OpenOptions::new` budgets.
#[cfg(test)]
pub(crate) fn fixture_deadline(fresh: u32, reopened: u32) -> std::time::Duration {
    let starts = fresh.saturating_mul(4).saturating_add(reopened);
    let single_stall = crate::server::close_budget()
        .max(crate::store::QUERY_TIMEOUT)
        .max(default_startup());
    server_start_budget()
        .saturating_mul(starts)
        .saturating_add(single_stall)
        .saturating_add(crate::store::QUERY_TIMEOUT)
}

#[cfg(test)]
mod fixture_deadline_tests {
    use super::fixture_deadline;
    use std::time::Duration;

    #[test]
    fn single_stall_defaults_match_the_reviewed_bounds() {
        for ((fresh, reopened), seconds) in
            [((1, 0), 190), ((1, 1), 222), ((2, 1), 350), ((2, 4), 446)]
        {
            assert_eq!(
                fixture_deadline(fresh, reopened),
                Duration::from_secs(seconds)
            );
        }
    }
}

/// Fixture options in the shared test cache, not yet warmed: a writable open
/// that would create the store fails at once until they are passed through
/// [`OpenOptions::warmed`]. Use [`warmed_open_options`] for a fixture that
/// creates its store; these suit reopens and path computations.
pub fn open_options(data_dir: PathBuf, project_scope: String) -> Result<OpenOptions> {
    let mut options = OpenOptions::new(data_dir, project_scope);
    options.config.cache_dir = Some(crate::store::test_cache());
    options.config.offline = true;
    options.supervisor = Some(crate::store::test_supervisor()?);
    options.fixture = Some(crate::store::Fixture::Unwarmed);
    Ok(options)
}

/// [`open_options`] after [`warm_runtime_cache`]: the shared engine and
/// store template are ready, so the fixture's open never builds either.
/// Call it before the fixture's deadline and before any spawn gate.
pub async fn warmed_open_options(data_dir: PathBuf, project_scope: String) -> Result<OpenOptions> {
    open_options(data_dir, project_scope)?.warmed().await
}

#[cfg(feature = "test-support")]
pub use crate::service::FixtureLoggedOwner;

/// Names an existing private file that receives the stderr of the owner a
/// process elects, when nothing else chooses it. A test sets it on each
/// command-line child it runs, since the electing child is the process that
/// reads it: the owner that child starts logs where the test can read it, and
/// the test never has to start that owner itself. It is deliberately not
/// forwarded to the owner, which elects nobody. Inert when unset.
#[cfg(feature = "test-support")]
pub use crate::service::OWNER_DIAGNOSTIC_ENV;

/// Owner open-activity hooks, read by the owner process itself, so a test
/// sets them on the command-line child whose owner it observes (Windows
/// owners receive them by explicit forwarding). Inert when unset.
#[cfg(feature = "test-support")]
pub use crate::service::activity::{OPEN_HOLD_DIR_ENV, WRITE_FAILURE_ENV};

/// Start the actual service executable with one caller-owned private stderr
/// file, and wait for its authenticated endpoint. The returned fixture holds
/// the attachment of the fixture that started it, which is the owner's
/// starter: the owner stays until that attachment and every other client have
/// released, and [`FixtureLoggedOwner::wait_for_exit`] releases it and awaits
/// the owner's own retirement. Start it only when no other owner is running,
/// and let no command-line child elect its own meanwhile; for the owner a
/// child elects, use [`OWNER_DIAGNOSTIC_ENV`].
#[cfg(feature = "test-support")]
pub async fn spawn_logged_owner(
    options: &OpenOptions,
    project: &Path,
    executable: &Path,
    diagnostic: File,
) -> Result<FixtureLoggedOwner> {
    // The spawned owner cannot carry a fixture token: warm here instead.
    warm_runtime_cache().await?;
    crate::service::spawn_logged_owner_fixture(options, project, executable, diagnostic).await
}

/// Seed a checked long public chain in one local fixture transaction before
/// an application process acquires the managed service's owner lease.
pub async fn seed_public_session(
    options: OpenOptions,
    catalog: &SessionCatalogRecord,
    turns: &[PublicTurnRecord],
) -> Result<()> {
    let store = crate::store::MemoryStore::open(options).await?;
    let insertion = store.fixture_insert_public_session(catalog, turns).await;
    let cleanup = store.close().await;
    insertion?;
    cleanup
}

/// Open an explicit real-engine fixture with its private startup log available on failure.
/// Ordinary `MemoryStore::open` retains its production error and log privacy behavior.
pub async fn open_fixture(options: OpenOptions) -> Result<MemoryStore> {
    let fixture_options = options.clone();
    MemoryStore::open(options)
        .await
        .map_err(|error| fixture_startup_error(&fixture_options, error))
}

/// Retire the exact managed owner once fixture clients have released their
/// transports. The owner retires by itself when its last client detaches, so
/// this waits behind one that is already closing, or asks a still-running one
/// (a starter-less or not-yet-reached owner) to retire. The maintenance permit
/// is dropped before a successor starts.
///
/// When its bound elapses, the error names the step the acquisition was
/// cancelled in, so a slow owner close, a still-attached client and a stalled
/// request are distinguishable from the failure text alone.
pub async fn retire_idle_service(options: &OpenOptions) -> Result<()> {
    let trace = crate::service::MaintenanceTrace::default();
    let mut refusals: u32 = 0;
    let permit = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match crate::service::acquire_maintenance_permit_traced(options, &trace).await {
                Ok(permit) => break Ok(permit),
                Err(error)
                    if error
                        .to_string()
                        .contains("memory service has active clients") =>
                {
                    refusals = refusals.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => break Err(error),
            }
        }
    })
    .await
    .with_context(|| {
        format!(
            "idle managed owner did not retire within 10 seconds; {trace}; active-client refusals={refusals}"
        )
    })??;
    drop(permit);
    Ok(())
}

/// Wait, without polling, until the managed owner for `options` has released
/// its owner lock: it retires its endpoint, reaps Dolt and only then unlocks,
/// so this observes the owner's exit from another process. Use it after the
/// last client of a spawned owner has closed, before asserting on its
/// retirement or electing a successor.
///
/// It has no deadline of its own; bound it with the fixture's backstop. If
/// the owner never retires, the blocked waiter cannot be cancelled and ends
/// with the process, so the test fails at its backstop. It briefly holds the
/// owner lock itself once the owner releases it, so call it only while no
/// other client is electing.
#[cfg(any(test, feature = "test-support"))]
pub async fn await_owner_release(options: &OpenOptions) -> Result<()> {
    crate::service::ServiceLock::await_release(
        &options.data_dir,
        &options.project_scope,
        crate::service::ServiceLockKind::Owner,
    )
    .await
}

/// Wait until a managed fixture's store has no live Dolt, and record that on
/// its fixture root, before the root is released.
///
/// The managed service retires as soon as its last client detaches, but it
/// closes asynchronously, so closing every client does not mean its engine is
/// already reaped. Call this after every client handle for `options` has
/// closed (an attached client makes [`retire_idle_service`] fail with "active
/// clients"). It waits behind the owner's own close, or retires an owner that
/// is still running, either of which releases the owner lock only after its
/// Dolt is reaped, then awaits [`await_store_quiescence`] for the project
/// store, each of its remaining staging directories and each stage preserved
/// under `interrupted/`. The product's asynchronous close is unchanged; only
/// fixtures wait for it. A fixture that does not know its projects finds
/// them with [`managed_store_scopes`].
pub async fn await_managed_quiescence(options: &OpenOptions) -> Result<()> {
    retire_idle_service(options).await?;
    let directory = crate::store::project_directory(&options.data_dir, &options.project_scope)?;
    #[cfg(unix)]
    let lifecycle_root: Option<PathBuf> = None;
    #[cfg(windows)]
    let lifecycle_root = Some(options.data_dir.join("memory/lifecycles"));
    for store in project_store_directories(&directory)? {
        await_store_quiescence(&store, lifecycle_root.as_deref())
            .await
            .with_context(|| {
                format!(
                    "managed fixture store {} kept a live Dolt after its owner retired",
                    store.display()
                )
            })?;
    }
    Ok(())
}

/// The project store, its `.staging-*` siblings and its stages preserved
/// under `interrupted/` that exist now: every directory in which a managed
/// open of this project may have run an engine.
fn project_store_directories(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut stores = Vec::new();
    if lifecycle_trace::exists(directory) {
        stores.push(directory.to_path_buf());
    }
    let (Some(parent), Some(name)) = (
        directory.parent(),
        directory.file_name().and_then(OsStr::to_str),
    ) else {
        return Ok(stores);
    };
    let prefix = format!("{name}.staging-");
    for location in [parent.to_path_buf(), parent.join("interrupted")] {
        let entries = match fs::read_dir(&location) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for (index, entry) in entries.enumerate() {
            ensure!(
                index < MAX_STAGE_ENTRIES,
                "too many entries beside managed fixture store {}",
                directory.display()
            );
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|entry| entry.starts_with(&prefix))
                && entry.file_type()?.is_dir()
            {
                stores.push(entry.path());
            }
        }
    }
    Ok(stores)
}

/// The project scopes that have a store directory under `data_dir` now, for
/// a fixture that must await [`await_managed_quiescence`] for every project a
/// managed service may have served, without knowing its projects.
///
/// A fresh open runs its engines in a `<hash>.staging-<uuid>` directory and
/// renames it only once activation is validated, so a store an owner in
/// another process is still opening, or left unactivated, exists only under
/// that name. A scope is included exactly when
/// [`await_managed_quiescence`]'s own recognition finds a store for it, so
/// the two cannot drift: an entry names a candidate scope by its text before
/// the first `.`, which the product's project directory rule must accept.
/// A missing memory root has no scopes; any other unreadable root is an error.
pub fn managed_store_scopes(data_dir: &Path) -> Result<Vec<String>> {
    let memory = data_dir.join("memory");
    let mut candidates = std::collections::BTreeSet::new();
    for location in [memory.clone(), memory.join("interrupted")] {
        let entries = match fs::read_dir(&location) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect fixture memory root {}", location.display())
                });
            }
        };
        for (index, entry) in entries.enumerate() {
            ensure!(
                index < MAX_STAGE_ENTRIES,
                "too many entries in fixture memory root {}",
                location.display()
            );
            let name = entry?.file_name();
            if let Some(candidate) = name.to_str().and_then(|name| name.split('.').next()) {
                candidates.insert(format!("project/{candidate}"));
            }
        }
    }
    let mut scopes = Vec::new();
    for scope in candidates {
        let Ok(directory) = crate::store::project_directory(data_dir, &scope) else {
            continue;
        };
        if !project_store_directories(&directory)?.is_empty() {
            scopes.push(scope);
        }
    }
    Ok(scopes)
}

/// Await the lifecycle lease of one store directory, bounded by the
/// supervisor's own reap allowance, and record on the fixture ledger, while
/// the lease is held, that no engine is live there.
///
/// This is the record [`TempDir`]'s teardown requires for any store whose
/// engine this process did not reap itself: a store served by a managed
/// service process, by a spawned `kuru` process, or a lease-only directory.
/// A timeout is returned as an error, so the fixture fails.
pub async fn await_store_quiescence(directory: &Path, lifecycle_root: Option<&Path>) -> Result<()> {
    await_store_quiescence_observed(directory, lifecycle_root, |_| ()).await
}

pub(crate) async fn await_store_quiescence_observed(
    directory: &Path,
    lifecycle_root: Option<&Path>,
    while_held: impl FnOnce(&crate::server::LifecycleLease),
) -> Result<()> {
    // A waiting acquisition, not a probe: a descriptor a sibling's child
    // inherited only delays it, and the fixture guard reads the record, not
    // this lock. It takes no spawn gate, so callers may hold `spawning()`.
    let lease = crate::server::Server::quiescence_at(
        directory,
        lifecycle_root,
        crate::server::SUPERVISOR_REAP_ALLOWANCE,
    )
    .await
    .with_context(|| format!("memory store {} did not quiesce", directory.display()))?;
    engine_ledger::record(lease.directory.path());
    while_held(&lease);
    drop(lease);
    Ok(())
}

#[cfg(test)]
pub(crate) async fn open_local_fixture(options: OpenOptions) -> Result<crate::store::MemoryStore> {
    let fixture_options = options.clone();
    crate::store::MemoryStore::open(options)
        .await
        .map_err(|error| fixture_startup_error(&fixture_options, error))
}

/// [`MemoryStore::open`], serialised against this lib's own advisory-lock
/// tests; see `crate::spawn_gate`. Every real-engine open in the workspace
/// behavioral fixtures (`store::recovery_tests`, `store::migration_lifecycle_tests`,
/// `store::operational_gc_tests`) goes through this single choke point instead
/// of the raw `MemoryStore::open` so the flock/posix_spawn race those fixtures'
/// own concurrent spawns can otherwise open stays excluded without gating each
/// call site by hand. Error text is passed through unchanged (unlike
/// `open_fixture`), so it stays a drop-in replacement for assertions on the
/// raw `MemoryStore::open` result.
#[cfg(test)]
pub(crate) async fn spawn_gated_open(options: OpenOptions) -> Result<crate::store::MemoryStore> {
    let _gate = crate::spawn_gate::spawning().await;
    crate::store::MemoryStore::open(options).await
}

pub(crate) fn fixture_startup_error(options: &OpenOptions, error: Error) -> Error {
    if !error.chain().any(|cause| {
        let message = cause.to_string();
        message.starts_with("Dolt startup/lifetime failed; private diagnostics:")
            || message.starts_with(
                "memory server startup failed: Dolt startup/lifetime failed; private diagnostics:",
            )
            || message.starts_with("Dolt database bootstrap deadline exceeded while ")
            || message == "authenticated Dolt startup deadline exceeded"
            || message.starts_with("fixture service exited before")
            || message == "memory supervisor readiness deadline exceeded"
    }) {
        return error;
    }
    match staged_fixture_server_log(options) {
        FixtureLog::Found(log) => error.context(log),
        FixtureLog::Absent => match active_fixture_server_log(options) {
            Some(log) => error.context(log),
            None => error,
        },
        FixtureLog::Unsafe => error,
    }
}

fn fixture_server_log(log: PathBuf) -> Option<String> {
    let bytes = files::read_bytes(&log, STARTUP_LOG_BYTES).ok()?;
    let tail = &bytes[bytes.len().saturating_sub(STARTUP_TAIL_BYTES)..];
    Some(format!(
        "fixture Dolt server log tail ({}): {}",
        log.display(),
        String::from_utf8_lossy(tail)
    ))
}

enum FixtureLog {
    Found(String),
    Absent,
    Unsafe,
}

fn staged_fixture_server_log(options: &OpenOptions) -> FixtureLog {
    let Ok(active) = crate::store::project_directory(&options.data_dir, &options.project_scope)
    else {
        return FixtureLog::Unsafe;
    };
    let Some(parent) = active.parent() else {
        return FixtureLog::Unsafe;
    };
    let Some(name) = active.file_name().and_then(|name| name.to_str()) else {
        return FixtureLog::Unsafe;
    };
    let prefix = format!("{name}.staging-");
    let mut stage = None;
    let Ok(entries) = fs::read_dir(parent) else {
        return FixtureLog::Unsafe;
    };
    for (index, entry) in entries.enumerate() {
        if index >= MAX_STAGE_ENTRIES {
            return FixtureLog::Unsafe;
        }
        let Ok(entry) = entry else {
            return FixtureLog::Unsafe;
        };
        let entry_name = entry.file_name();
        let Some(suffix) = entry_name
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
        else {
            continue;
        };
        if uuid::Uuid::parse_str(suffix).is_err() {
            continue;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_dir())
            || stage.replace(entry.path()).is_some()
        {
            return FixtureLog::Unsafe;
        }
    }
    let Some(stage) = stage else {
        return FixtureLog::Absent;
    };
    let log = stage.join("server.log");
    match fs::symlink_metadata(&log) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FixtureLog::Absent,
        Ok(_) => fixture_server_log(log).map_or(FixtureLog::Unsafe, FixtureLog::Found),
        Err(_) => FixtureLog::Unsafe,
    }
}

fn active_fixture_server_log(options: &OpenOptions) -> Option<String> {
    fixture_server_log(
        crate::store::project_directory(&options.data_dir, &options.project_scope)
            .ok()?
            .join("server.log"),
    )
}

#[cfg(test)]
mod fixture_diagnostic_tests {
    use super::*;

    fn startup_error() -> Error {
        anyhow::anyhow!(
            "memory server startup failed: Dolt startup/lifetime failed; private diagnostics: fixture/server.log: Dolt exited before readiness"
        )
    }

    fn readiness_error() -> Error {
        anyhow::anyhow!("memory supervisor readiness deadline exceeded")
            .context("memory startup cleanup also failed: memory supervisor exited unsuccessfully (exit code: 1)")
            .context("open staged memory server")
    }

    fn bootstrap_error() -> Error {
        anyhow::anyhow!(
            "Dolt database bootstrap deadline exceeded while verifying the project identity"
        )
    }

    #[test]
    fn startup_log_capture_is_opt_in_exact_and_bounded() -> Result<()> {
        let root = tempdir()?;
        let options = OpenOptions::new(root.path().join("private"), format!("project/{:064x}", 7));
        let active = crate::store::project_directory(&options.data_dir, &options.project_scope)?;
        let stage = active.with_file_name(format!(
            "{}.staging-{}",
            active
                .file_name()
                .context("fixture project name missing")?
                .to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        files::private_dir(&stage)?;
        let log = stage.join("server.log");
        files::write(&log, b"fixture-private-log")?;

        let unrelated = anyhow::anyhow!("ordinary fixture error");
        let unchanged = fixture_startup_error(&options, unrelated);
        assert_eq!(unchanged.to_string(), "ordinary fixture error");
        let unrelated_deadline = fixture_startup_error(
            &options,
            anyhow::anyhow!("provider readiness deadline exceeded"),
        );
        assert_eq!(
            unrelated_deadline.to_string(),
            "provider readiness deadline exceeded"
        );
        let captured = fixture_startup_error(&options, startup_error());
        let rendered = format!("{captured:#}");
        assert!(rendered.contains("fixture-private-log"));
        assert!(rendered.contains(&log.display().to_string()));
        assert!(rendered.contains("Dolt exited before readiness"));
        let readiness_captured = fixture_startup_error(&options, readiness_error());
        let readiness_rendered = format!("{readiness_captured:#}");
        assert!(readiness_rendered.contains("fixture-private-log"));
        let exited = fixture_startup_error(
            &options,
            anyhow::anyhow!("fixture service exited before readiness: exit code: 1"),
        );
        assert!(format!("{exited:#}").contains("fixture-private-log"));
        assert!(readiness_rendered.contains(&log.display().to_string()));
        assert!(readiness_rendered.contains("memory supervisor readiness deadline exceeded"));
        let bootstrap_captured = fixture_startup_error(&options, bootstrap_error());
        let bootstrap_rendered = format!("{bootstrap_captured:#}");
        assert!(bootstrap_rendered.contains("fixture-private-log"));
        assert!(bootstrap_rendered.contains("verifying the project identity"));

        files::write(&log, &vec![b'x'; 8 * 1024])?;
        let bounded = fixture_startup_error(&options, startup_error()).to_string();
        assert!(bounded.len() < 5 * 1024, "fixture log tail was not bounded");
        assert!(bounded.ends_with(&"x".repeat(4 * 1024)));
        fs::remove_file(&log)?;
        files::private_dir(&active)?;
        let active_log = active.join("server.log");
        files::write(&active_log, b"fixture-active-log")?;
        let active_captured = fixture_startup_error(&options, startup_error());
        assert!(format!("{active_captured:#}").contains("fixture-active-log"));
        let parent = active.parent().context("active project parent")?;
        let name = active
            .file_name()
            .context("active project name")?
            .to_string_lossy();
        for _ in 0..2 {
            files::private_dir(&parent.join(format!("{name}.staging-{}", uuid::Uuid::new_v4())))?;
        }
        let ambiguous = fixture_startup_error(&options, startup_error());
        assert!(
            !format!("{ambiguous:#}").contains("fixture-active-log"),
            "ambiguous staging must not disclose an active log"
        );
        fs::remove_file(&active_log)?;
        let missing = fixture_startup_error(&options, startup_error());
        assert!(!format!("{missing:#}").contains("fixture Dolt server log tail"));
        Ok(())
    }

    #[tokio::test]
    async fn ordinary_open_keeps_its_error_with_test_support_compiled() -> Result<()> {
        let root = tempdir()?;
        let options = OpenOptions::new(root.path().join("private"), "invalid scope".into());
        let error = MemoryStore::open(options).await.err();
        let error = error.context("ordinary fixture open unexpectedly succeeded")?;
        assert!(format!("{error:#}").contains("memory project scope must start with project/"));
        assert!(!format!("{error:#}").contains("fixture Dolt server log tail"));
        Ok(())
    }
}

#[cfg(test)]
mod managed_store_scope_tests {
    use super::*;

    fn hash(digit: char) -> String {
        digit.to_string().repeat(64)
    }

    #[test]
    fn scopes_cover_active_staging_and_preserved_stores_only() -> Result<()> {
        let root = tempdir()?;
        let data = root.path().join("private");
        assert!(managed_store_scopes(&data)?.is_empty(), "no memory root");
        let memory = data.join("memory");
        for directory in ["locks", "lifecycles", "interrupted"] {
            files::private_dir(&memory.join(directory))?;
        }
        let uuid = uuid::Uuid::new_v4();
        // Active, staging-only (an open another process had not activated)
        // and preserved-only stores each name their scope.
        files::private_dir(&memory.join(hash('1')))?;
        files::private_dir(&memory.join(format!("{}.staging-{uuid}", hash('2'))))?;
        files::private_dir(&memory.join(format!("interrupted/{}.staging-{uuid}", hash('3'))))?;
        // Neither an invalid project digest, nor a staging-named file, nor
        // another suffix of a digest with no store names a scope.
        files::private_dir(&memory.join("F".repeat(64)))?;
        files::write(
            &memory.join(format!("{}.staging-{uuid}", hash('4'))),
            b"not a store",
        )?;
        files::private_dir(&memory.join(format!("{}.purge-{uuid}-0", hash('5'))))?;
        assert_eq!(
            managed_store_scopes(&data)?,
            ['1', '2', '3']
                .map(|digit| format!("project/{}", hash(digit)))
                .to_vec()
        );
        // The staging-only and preserved-only stores are exactly what
        // `await_managed_quiescence` awaits for their scopes.
        let staged = crate::store::project_directory(&data, &format!("project/{}", hash('2')))?;
        assert_eq!(
            project_store_directories(&staged)?,
            vec![memory.join(format!("{}.staging-{uuid}", hash('2')))]
        );
        let preserved = crate::store::project_directory(&data, &format!("project/{}", hash('3')))?;
        assert_eq!(
            project_store_directories(&preserved)?,
            vec![memory.join(format!("interrupted/{}.staging-{uuid}", hash('3')))]
        );

        let unreadable = root.path().join("unreadable");
        files::private_dir(&unreadable)?;
        files::write(&unreadable.join("memory"), b"not a memory root")?;
        let error = managed_store_scopes(&unreadable).unwrap_err();
        assert!(
            format!("{error:#}").contains("inspect fixture memory root"),
            "{error:#}"
        );
        Ok(())
    }
}

/// Commit one invalid state payload for an isolated export-failure fixture.
pub async fn commit_malformed_state(store: &crate::MemoryStore, key: &str) -> Result<()> {
    store.fixture_commit_malformed_state(key).await
}

/// The shared test cache, for a spawned process to use, after warming its
/// engine and store template synchronously ([`warm_blocking`]). A caller
/// inside a Tokio runtime uses [`warmed_cache_dir`] instead.
pub fn cache_dir() -> Result<PathBuf> {
    warm_blocking()?;
    Ok(crate::store::test_cache())
}

/// [`cache_dir`] for a caller inside a Tokio runtime.
pub async fn warmed_cache_dir() -> Result<PathBuf> {
    warm_runtime_cache().await?;
    Ok(crate::store::test_cache())
}

/// Threads [`warm_blocking`] started, for its own test.
#[cfg(test)]
pub(crate) static WARM_THREADS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Run [`warm_runtime_cache`] to completion from synchronous code, on a
/// private thread with its own current-thread runtime. Refuses to run inside
/// a Tokio runtime, where blocking could starve another task's in-flight
/// warm-up: such a caller awaits [`warmed_cache_dir`].
pub fn warm_blocking() -> Result<()> {
    ensure!(
        tokio::runtime::Handle::try_current().is_err(),
        "test_support::warm_blocking cannot run inside a Tokio runtime; await \
         test_support::warmed_cache_dir() instead"
    );
    #[cfg(test)]
    WARM_THREADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(warm_runtime_cache())
            .map(|_| ())
    })
    .join()
    .map_err(|_| anyhow::anyhow!("the test cache warm-up thread panicked"))?
}

fn profile(executable: &Path) -> Result<&Path> {
    let parent = executable
        .parent()
        .context("test executable has no parent")?;
    if parent.file_name().is_some_and(|name| name == "deps") {
        parent
            .parent()
            .context("test executable has no profile directory")
    } else {
        Ok(parent)
    }
}

fn digest(file: &mut File) -> Result<(u64, String)> {
    file.seek(SeekFrom::Start(0))?;
    let mut limited = file.take(LIMIT + 1);
    let mut hash = Sha256::new();
    let mut count = 0;
    let mut buffer = [0; 65536];
    loop {
        let read = limited.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        hash.update(&buffer[..read]);
    }
    ensure!(
        count > 0 && count <= LIMIT,
        "supervisor fixture exceeds size limit"
    );
    let hash = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((count, hash))
}

fn verified(directory: &Directory, receipt: &Receipt) -> Result<PathBuf> {
    let name = receipt.name()?;
    let mut file = directory.read(OsStr::new(&name))?;
    let actual = digest(&mut file)?;
    ensure!(
        actual == (receipt.bytes, receipt.sha256.clone()),
        "prepared supervisor checksum mismatch"
    );
    directory.verify(OsStr::new(&name), &file)?;
    Ok(directory.path().join(name))
}

/// Snapshot a trusted compiled fixture into its private profile-local cache.
/// This never adopts downloaded executables or modifies Cargo-owned artifacts.
pub fn snapshot_supervisor(source: &Path, profile: &Path) -> Result<PathBuf> {
    ensure!(
        source.is_absolute() && profile.is_absolute(),
        "fixture paths must be absolute"
    );
    ensure!(
        fs::symlink_metadata(source)?.file_type().is_file(),
        "compiled supervisor is not a regular file"
    );
    // Cargo legitimately hard-links the top-level alias to a compiled artifact.
    // Retain that source handle; snapshots themselves have one private inode.
    let mut input = File::open(source).context("open compiled supervisor before preparation")?;
    ensure!(
        input.metadata()?.is_file(),
        "compiled supervisor is not a regular file"
    );
    let (bytes, sha256) = digest(&mut input)?;
    let receipt = Receipt {
        schema_version: 1,
        sha256,
        bytes,
    };
    let directory = Directory::ensure_private(&profile.join(DIRECTORY))?;
    let lease = directory.lock_file(OsStr::new("prepare.lock"))?;
    lease.lock()?;
    directory.verify(OsStr::new("prepare.lock"), &lease)?;
    let name = receipt.name()?;
    match fs::symlink_metadata(directory.path().join(&name)) {
        Ok(_) => {
            verified(&directory, &receipt)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let stage = TempDir::new("supervisor-", Some(directory.path()))?;
            let staged = Directory::open(stage.path(), Privacy::OwnerOnly, NameRetention::Movable)?;
            let mut candidate = staged.create_new(OsStr::new("candidate"))?;
            input.seek(SeekFrom::Start(0))?;
            let copied = std::io::copy(&mut input.take(LIMIT + 1), &mut candidate)?;
            ensure!(
                copied == receipt.bytes,
                "compiled supervisor changed during snapshot"
            );
            ensure!(
                digest(&mut candidate)? == (receipt.bytes, receipt.sha256.clone()),
                "compiled supervisor changed during snapshot"
            );
            seal_private(&candidate, true)?;
            directory.publish_file(
                &staged,
                OsStr::new("candidate"),
                &candidate,
                OsStr::new(&name),
                Publication::New,
            )?;
        }
        Err(error) => return Err(error.into()),
    }
    let result = verified(&directory, &receipt)?;
    files::write(
        &directory.path().join("current.json"),
        &serde_json::to_vec(&receipt)?,
    )?;
    Ok(result)
}

/// Called by the owning prefetch task before dependent ordinary tests start.
pub fn prepare_supervisor() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    snapshot_supervisor(&executable, profile(&executable)?)
}

fn from_profile(profile: &Path) -> Result<PathBuf> {
    let directory = Directory::open(
        &profile.join(DIRECTORY),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )?;
    let bytes = files::read_bytes(&directory.path().join("current.json"), 1024)?;
    let receipt: Receipt = serde_json::from_slice(&bytes)?;
    verified(&directory, &receipt)
}

pub(crate) fn prepared_supervisor() -> Result<Option<PathBuf>> {
    let Some(value) = std::env::var_os("KURU_TEST_SUPERVISOR_PREPARED") else {
        return Ok(None);
    };
    ensure!(
        value == "1",
        "KURU_TEST_SUPERVISOR_PREPARED must be 1 or unset"
    );
    static SNAPSHOT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    match SNAPSHOT.get_or_init(|| {
        let result = (|| from_profile(profile(&std::env::current_exe()?)?))();
        result.map_err(|error: anyhow::Error| format!("prepared Dolt supervisor unavailable; run mise run //packages/kuru-memory:prefetch: {error:#}"))
    }) {
        Ok(path) => Ok(Some(path.clone())),
        Err(message) => anyhow::bail!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_warm_up_is_shared_and_returns_the_cached_engine() -> Result<()> {
        let (first, second) = tokio::join!(warm_runtime_cache(), warm_runtime_cache());
        let (first, second) = (first?, second?);
        ensure!(
            first == second,
            "concurrent warm-ups returned different engines"
        );
        ensure!(
            first.starts_with(crate::store::test_cache().canonicalize()?),
            "warm-up provisioned outside the shared test cache"
        );
        ensure!(
            first.is_file(),
            "warm-up did not return an installed engine"
        );
        ensure!(
            warm_runtime_cache().await? == first,
            "a later warm-up did not reuse the process result"
        );
        Ok(())
    }

    #[test]
    fn snapshots_survive_source_removal_and_replacement_and_reject_corrupt_private_bytes() {
        // Held for the whole test: every `snapshot_supervisor` call below
        // acquires and releases a real flock (`prepare.lock`), and this test
        // never itself spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking();
        let root = tempdir().unwrap();
        let source = root.path().join("cargo alias");
        fs::write(&source, b"first compiled fixture").unwrap();
        let first = snapshot_supervisor(&source, root.path()).unwrap();
        fs::remove_file(&source).unwrap();
        assert_eq!(from_profile(root.path()).unwrap(), first);
        assert_eq!(fs::read(&first).unwrap(), b"first compiled fixture");
        fs::write(&source, b"second compiled fixture").unwrap();
        let second = snapshot_supervisor(&source, root.path()).unwrap();
        assert_ne!(first, second);
        assert_eq!(from_profile(root.path()).unwrap(), second);
        assert_eq!(fs::read(&first).unwrap(), b"first compiled fixture");
        // Prepared executable files are sealed. Model a replaced cache object,
        // not permission to mutate that retained immutable inode in place.
        fs::remove_file(&second).unwrap();
        files::write(&second, b"corrupted private bytes").unwrap();
        assert!(from_profile(root.path()).is_err());
        assert!(snapshot_supervisor(&source, root.path()).is_err());
        assert_eq!(fs::read(&second).unwrap(), b"corrupted private bytes");
    }

    /// An owner lock held with no endpoint published, as by an owner still
    /// reaping Dolt after retiring its record: the fixture's bound elapses
    /// and its error names the step it was cancelled in and what the
    /// retirement requests met, not only that the deadline elapsed. Paused
    /// time advances the fixture's own waits; nothing here waits on a clock.
    #[tokio::test(start_paused = true)]
    async fn an_elapsed_retirement_bound_names_the_step_it_was_cancelled_in() -> Result<()> {
        let root = tempdir()?;
        let options = open_options(
            root.path().join("private"),
            format!("project/{}", "1".repeat(64)),
        )?;
        let owner = crate::service::ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            crate::service::ServiceLockKind::Owner,
        )?
        .context("a fresh fixture's owner lock was busy")?;
        let error = retire_idle_service(&options)
            .await
            .expect_err("retirement completed while the owner lock was held");
        let text = format!("{error:#}");
        ensure!(
            text.contains("idle managed owner did not retire within 10 seconds")
                && text.contains("maintenance waiting for the owner lock for ")
                && text.contains("busy replies=0")
                && text.contains("active-client refusals=0")
                && !text.contains("requests without a live endpoint=0;"),
            "the elapsed bound did not name its step: {text}"
        );
        owner.release()?;
        root.release(Ok(()))
    }

    /// A project that never had an owner: its owner lock is free, so the
    /// wait returns at once, and leaves the lock free.
    #[tokio::test]
    async fn awaiting_the_release_of_an_owner_lock_nobody_holds_returns_at_once() -> Result<()> {
        let root = tempdir()?;
        let options = open_options(
            root.path().join("private"),
            format!("project/{}", "0".repeat(64)),
        )?;
        tokio::time::timeout(Duration::from_secs(30), await_owner_release(&options))
            .await
            .context("the wait for an unheld owner lock did not return")??;
        ensure!(
            crate::service::ServiceLock::try_acquire(
                &options.data_dir,
                &options.project_scope,
                crate::service::ServiceLockKind::Owner,
            )?
            .map(crate::service::ServiceLock::release)
            .transpose()?
            .is_some(),
            "the wait left the owner lock held"
        );
        Ok(())
    }

    /// A really spawned owner: the wait returns only when the last client's
    /// detach has made the owner retire its endpoint, reap Dolt and release
    /// its lock, without any timer or poll on the test's side.
    #[tokio::test]
    async fn awaiting_the_release_of_a_spawned_owner_observes_its_retirement() -> Result<()> {
        warm_runtime_cache().await?;
        // Real lifecycles: one fresh elected service process.
        let deadline = fixture_deadline(1, 0);
        let root = tempdir()?;
        let project = root.path().join("project");
        fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let scope = format!(
            "project/{}",
            Sha256::digest(project.as_os_str().as_encoded_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let options = open_options(root.path().join("private"), scope)?;
        let executable = crate::store::test_supervisor()?;
        let gate = crate::spawn_gate::spawning().await;
        let waited = tokio::time::timeout(deadline, async {
            let attachment = crate::service::attach_or_start(&options, &project, &executable)
                .await
                .context("elect a spawned owner")?;
            let generation = attachment.generation().to_owned();
            ensure!(
                crate::service::EndpointRecord::read(&options.data_dir, &options.project_scope)?
                    .is_some_and(|record| record.authority.service_generation == generation),
                "the spawned owner did not publish its endpoint"
            );
            drop(attachment);
            await_owner_release(&options).await?;
            ensure!(
                crate::service::EndpointRecord::read(&options.data_dir, &options.project_scope)?
                    .is_none(),
                "the owner released its lock without retiring its endpoint"
            );
            Ok::<_, Error>(())
        })
        .await
        .with_context(|| format!("spawned owner fixture exceeded its {deadline:?} deadline"))?;
        drop(gate);
        waited?;
        let directory = crate::store::project_directory(&options.data_dir, &options.project_scope)?;
        #[cfg(unix)]
        let lifecycle_root: Option<PathBuf> = None;
        #[cfg(windows)]
        let lifecycle_root = Some(options.data_dir.join("memory/lifecycles"));
        for store in project_store_directories(&directory)? {
            await_store_quiescence(&store, lifecycle_root.as_deref()).await?;
        }
        root.release(Ok(()))
    }
}
