//! Isolated real-engine fixtures shared by workspace behavioral tests.
//!
//! Cargo owns and can replace its profile's top-level executable alias while
//! another package is already testing. Ordinary mise test tasks explicitly use
//! the snapshot prepared before they start. Coverage deliberately does not opt
//! in: its one workspace build supplies the actual instrumented executable.
pub use crate::files::PrivateTemp as TempDir;
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
/// before the client reads its reply. The owner continues independently.
#[derive(Clone, Default)]
pub struct ReplyBarrier {
    pub(crate) inner: Arc<crate::service::rpc::ReplyPause>,
}

impl ReplyBarrier {
    pub async fn wait_sent(&self) {
        self.inner.sent.notified().await;
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

/// Provision the bundled Dolt runtime into the shared test cache once per test
/// process and return its executable.
///
/// Fixtures call this before starting their own outer deadline, so a cold
/// install is paid here once instead of inside whichever fixtures happen to
/// start first. The wait is bounded by the provisioner's cache-lock peer budget
/// plus its version-probe budget and [`WARM_UP_MARGIN`], so the product's own
/// lock error wins that race and an installing caller's extraction and probe
/// are covered; the result, including a failure, is shared by every later
/// caller in the process.
pub async fn warm_runtime_cache() -> Result<PathBuf> {
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

/// Allowance for creating a fixture child process and its runtime before the
/// product's own startup clock begins inside it.
#[cfg(test)]
pub(crate) const CHILD_START_MARGIN: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(test)]
fn default_startup() -> std::time::Duration {
    let config = OpenOptions::new(PathBuf::new(), String::new()).config;
    std::time::Duration::from_secs(config.startup_timeout_secs)
}

/// One owned Dolt server start: the configured startup timeout plus the
/// supervisor-transport allowance `Server::open` adds to it.
#[cfg(test)]
pub(crate) fn server_start_budget() -> std::time::Duration {
    default_startup().saturating_add(crate::server::SUPERVISOR_TRANSPORT_ALLOWANCE)
}

/// A fresh store open (`MemoryStore::open_inner` with no active directory):
/// the startup lock wait, four server starts (initialization, migration,
/// validation, active) each with one `QUERY_TIMEOUT` session, three owned
/// server closes, and the staged directory's quiescence wait.
#[cfg(test)]
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
/// `QUERY_TIMEOUT` for its own operations. A fixture whose owner retires only
/// through idle expiry adds `SERVICE_IDLE_TIMEOUT` at the call site.
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

pub fn open_options(data_dir: PathBuf, project_scope: String) -> Result<OpenOptions> {
    let mut options = OpenOptions::new(data_dir, project_scope);
    options.config.cache_dir = Some(crate::store::test_cache());
    options.config.offline = true;
    options.supervisor = Some(crate::store::test_supervisor()?);
    Ok(options)
}

#[cfg(feature = "test-support")]
pub use crate::service::FixtureLoggedOwner;

/// Start the actual service executable with one caller-owned private stderr
/// file, then wait for its authenticated endpoint before CLI children attach.
#[cfg(feature = "test-support")]
pub async fn spawn_logged_owner(
    options: &OpenOptions,
    project: &Path,
    executable: &Path,
    diagnostic: File,
) -> Result<FixtureLoggedOwner> {
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

/// Retire the exact idle managed owner after fixture clients release their
/// transports. The maintenance permit is dropped before a successor starts.
pub async fn retire_idle_service(options: &OpenOptions) -> Result<()> {
    let permit = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match crate::service::acquire_maintenance_permit(options).await {
                Ok(permit) => break Ok(permit),
                Err(error)
                    if error
                        .to_string()
                        .contains("memory service has active clients") =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => break Err(error),
            }
        }
    })
    .await
    .context("idle managed owner did not retire within 10 seconds")??;
    drop(permit);
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

/// Commit one invalid state payload for an isolated export-failure fixture.
pub async fn commit_malformed_state(store: &crate::MemoryStore, key: &str) -> Result<()> {
    store.fixture_commit_malformed_state(key).await
}

pub fn cache_dir() -> PathBuf {
    crate::store::test_cache()
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
}
