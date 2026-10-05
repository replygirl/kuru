//! Bounded, private installation of the exact supported full-Dolt engine.

use crate::MemoryOpenStage;
pub use crate::catalog::DOLT_VERSION;
use crate::catalog::{Asset, BUNDLED_ASSET, EMBEDDED_ARCHIVE, MAX_COMPRESSED, MAX_EXPANDED};
use crate::files::{self, PrivateTemp, StageCleanupFailure};
use crate::progress::{ByteTicks, OpenTicks, ProgressReporter, TickingWriter};
use anyhow::{Context, Result, bail, ensure};
use flate2::bufread::GzDecoder;
use kuru_core::MemoryConfig;
use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info, seal_private};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    borrow::Cow,
    collections::HashSet,
    fs::{self, File, TryLockError},
    io::{Cursor, Read, Seek},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};
#[cfg(unix)]
use tokio::process::Command;

mod probe_diagnostics;
use probe_diagnostics::{PipeFacts, ProbeDiagnostics};

pub(crate) const LOCK_TIMEOUT: Duration = Duration::from_secs(180);
pub(crate) const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: u64 = 4096;
#[cfg(windows)]
const ACTIVATION_RETRY_LIMIT: Duration = Duration::from_secs(2);
#[cfg(windows)]
const ACTIVATION_RETRY_SPACING: Duration = Duration::from_millis(20);

/// Extract the bundled engine or reuse its verified cache, including offline
/// first use. An explicit development binary still passes the exact version
/// guard. Managed entries pass their immutable payload checksums on every open;
/// their exact version is probed once, before a fresh extraction is activated.
pub async fn provision(config: &MemoryConfig, default_cache: &Path) -> Result<PathBuf> {
    let mut progress = ProgressReporter::silent();
    provision_observed(config, default_cache, &mut progress).await
}

pub(crate) async fn provision_observed(
    config: &MemoryConfig,
    default_cache: &Path,
    progress: &mut ProgressReporter,
) -> Result<PathBuf> {
    config.validate()?;
    if let Some(binary) = &config.dolt_binary {
        checked_regular(binary, true)?;
        let binary = binary
            .canonicalize()
            .context("resolve configured Dolt executable")?;
        progress.report(MemoryOpenStage::CheckingRuntimeVersion);
        private_probe(binary.clone()).await?;
        return Ok(binary);
    }
    provision_managed_observed(
        config,
        default_cache,
        BUNDLED_ASSET,
        Cow::Borrowed(EMBEDDED_ARCHIVE),
        progress,
    )
    .await
}

#[cfg(all(test, unix))]
async fn provision_managed(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
) -> Result<PathBuf> {
    let mut progress = ProgressReporter::silent();
    provision_managed_observed(config, default_cache, asset, archive, &mut progress).await
}

async fn provision_managed_observed(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
    progress: &mut ProgressReporter,
) -> Result<PathBuf> {
    // The extraction advances this open's counter per 8 MiB written.
    let ticks = progress.ticks().cloned();
    let extractor = move |archive: &[u8], destination: &Path, asset: Asset<'static>| {
        extract_ticking(archive, destination, asset, ticks.as_ref())
    };
    provision_with_extractor_observed(config, default_cache, asset, archive, extractor, progress)
        .await
}

#[cfg(all(test, unix))]
async fn provision_with_extractor(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
    extractor: impl FnOnce(&[u8], &Path, Asset<'static>) -> Result<()> + Send + 'static,
) -> Result<PathBuf> {
    let mut progress = ProgressReporter::silent();
    provision_with_extractor_observed(
        config,
        default_cache,
        asset,
        archive,
        extractor,
        &mut progress,
    )
    .await
}

async fn provision_with_extractor_observed(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
    extractor: impl FnOnce(&[u8], &Path, Asset<'static>) -> Result<()> + Send + 'static,
    progress: &mut ProgressReporter,
) -> Result<PathBuf> {
    let cache = config.cache_dir.as_deref().unwrap_or(default_cache);
    private_directory(cache)?;
    let cache = cache.canonicalize()?;
    let versions = cache.join(DOLT_VERSION);
    private_directory(&versions)?;
    let destination = versions.join(asset.target);
    if destination_exists(&destination)? {
        // The common case: an already-published engine. Sweeping here is the
        // only way a leftover stage from a past publication is ever
        // collected without a fresh install, so it must stay opportunistic -
        // no lock is taken unless a receipt exists, and none is waited for.
        warm_sweep_if_receipted(cache, versions).await;
        progress.report(MemoryOpenStage::VerifyingRuntimeCache);
        return verify_existing_cache(&destination, asset, progress.ticks()).await;
    }
    progress.report(MemoryOpenStage::WaitingForRuntimeCache);
    let lock = cache_lock(&cache, LOCK_TIMEOUT).await?;
    // Collect any stage a past publication receipted before this installer
    // creates its own; the exclusive lock already held here is exactly the
    // serialization point that makes a swept stage safe to remove.
    report_leftover_stage_cap(sweep_leftover_stages(&versions, &lock));
    if destination_exists(&destination)? {
        drop(lock);
        progress.report(MemoryOpenStage::VerifyingRuntimeCache);
        return verify_existing_cache(&destination, asset, progress.ticks()).await;
    }
    // From here the stage and the lock travel as one lease, so every exit -
    // an error, a cancelled caller or a worker that outlives it - resolves
    // the stage before the lock is released (see `StageLease`).
    let lease = StageLease::new(PrivateTemp::new(".install-", Some(&versions))?, lock, asset);
    progress.report(MemoryOpenStage::ExtractingEmbeddedRuntime);
    crate::open_timeline::stamp(crate::open_timeline::Event::ExtractStart);
    let candidate = lease.path().join("runtime");
    let candidate_path = candidate.clone();
    let (lease, extraction) = tokio::task::spawn_blocking(move || {
        let result = extractor(&archive, &candidate_path, asset);
        // Keep the lease until the worker exits, even if its caller is
        // cancelled; a dropped lease resolves its stage here, after the last
        // write, and only then releases the lock.
        (lease, result)
    })
    .await?;
    crate::open_timeline::stamp(crate::open_timeline::Event::ExtractEnd);
    if let Err(error) = extraction {
        return Err(lease.discard_after(error));
    }
    progress.report(MemoryOpenStage::CheckingRuntimeVersion);
    let probe_home = lease.path().join("probe");
    let candidate_path = candidate.clone();
    let probe_path = probe_home.clone();
    let probe_ticks = progress.ticks().cloned();
    let (probe, lease) = tokio::task::spawn_blocking(move || {
        // On cancellation, the completed output drops in this field order:
        // the checked probe, then the lease (its stage, then the lock).
        (
            prepare_cold_probe_with(
                &candidate_path,
                &probe_path,
                asset,
                probe_ticks.as_ref(),
                |_| Ok(()),
            ),
            lease,
        )
    })
    .await?;
    let probe = match probe {
        Ok(probe) => probe,
        Err(error) => return Err(lease.discard_after(error)),
    };
    let (probe, lease) = probe.probe(lease).await?;
    let retained = activate_staged_after_probe(lease, probe, &candidate, &destination).await?;
    if let Some(report) = retained {
        // The engine is published and verified. Its lease already receipted
        // and reported the stage that outlived its bounded removal, before
        // releasing the lock; that never fails this open. A stage whose
        // receipt could not be written is not collectable by any later sweep,
        // so it is never reported as waiting for one.
        progress.report(if report.receipt_error.is_some() {
            MemoryOpenStage::RetainedUnreceiptedInstallStage
        } else {
            MemoryOpenStage::RetainedInstallStage
        });
    }
    Ok(destination.join(asset.executable_name))
}

/// Receipts naming the retained install stages of one engine version.
const LEFTOVER_STAGE_RECEIPTS: &str = ".leftovers";

/// What was retained when an installation could not remove its own private
/// stage: where it is, why it stayed, and which engine it belongs to.
/// `published` distinguishes a published and verified engine from an
/// installation that failed, was cancelled or unwound before publication.
/// `attempts` and `elapsed` describe the bounded recovery window that actually
/// ran, and are absent when the cause is not that exhaustion.
/// `descendant` names the entry whose checked removal refused, relative to
/// `stage`, when the cause carries one. `probe_child` is the probed engine's
/// child process and whether its process object was still open when the
/// refusal was recorded, or the cause its stamp failed with (Windows only; see
/// [`ProbeChildObservation`]).
#[derive(Debug)]
pub(crate) struct StageCleanupReport {
    pub stage: PathBuf,
    pub private: PathBuf,
    pub engine_version: &'static str,
    pub target: String,
    pub executable_sha256: String,
    pub published: bool,
    pub first_cause: String,
    pub os_error: Option<i32>,
    pub attempts: Option<u32>,
    pub elapsed: Option<Duration>,
    pub descendant: Option<String>,
    pub probe_child: Option<ProbeChildObservation>,
    pub receipt_error: Option<String>,
}

impl StageCleanupReport {
    fn new(failure: StageCleanupFailure, asset: Asset<'_>) -> Self {
        let StageCleanupFailure {
            stage,
            private,
            cause,
        } = failure;
        let exhausted = cause.downcast_ref::<crate::files::StageCleanupExhausted>();
        Self {
            engine_version: DOLT_VERSION,
            target: asset.target.to_owned(),
            executable_sha256: asset.executable_sha256.to_owned(),
            published: true,
            attempts: exhausted.map(|exhausted| exhausted.attempts),
            elapsed: exhausted.map(|exhausted| exhausted.elapsed),
            os_error: os_error(&cause),
            descendant: refusing_descendant(&stage, &cause),
            probe_child: None,
            first_cause: format!("{cause:#}"),
            stage,
            private,
            receipt_error: None,
        }
    }
}

/// The first native OS error in a cause chain.
fn os_error(cause: &anyhow::Error) -> Option<i32> {
    cause
        .chain()
        .filter_map(|error| error.downcast_ref::<std::io::Error>())
        .find_map(std::io::Error::raw_os_error)
}

/// The descendant a checked tree removal in `cause` names, relative to
/// `stage`. A lease removes the stage's `private` child, so its descendant is
/// rebased onto the stage the receipt and every later sweep name.
fn refusing_descendant(stage: &Path, cause: &anyhow::Error) -> Option<String> {
    let removal = cause
        .chain()
        .find_map(|error| error.downcast_ref::<kuru_platform::fs::RemovalError>())?;
    let descendant = removal.path.join(removal.descendant.as_ref()?);
    let relative = descendant.strip_prefix(stage).unwrap_or(&descendant);
    Some(relative.to_string_lossy().into_owned())
}

/// The cold probe's child process, recorded on the stage lease so that a
/// refused removal can say whether the child's exited process object was
/// still open. On Windows that object keeps the image section of the executed
/// `probe` copy referenced, which refuses the copy's deletion. Only Windows
/// records it: an exited process never blocks an unlink on Unix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProbeChild {
    pub pid: u32,
    /// Creation time in FILETIME ticks, which tells a reused id apart.
    pub created: u64,
}

impl ProbeChild {
    /// `retained` while a process object with exactly this identity can still
    /// be opened (some handle, ours or another process's, keeps it alive),
    /// `released` once none can, or `unknown: <cause>`. Diagnostics only: it
    /// never decides how a stage is resolved and never authorizes acting on
    /// the process.
    fn state(self) -> String {
        #[cfg(windows)]
        {
            use kuru_platform::windows::process::{ProcessStamp, process_object_retained};
            match process_object_retained(ProcessStamp {
                id: self.pid,
                created: self.created,
            }) {
                Ok(true) => "retained".to_owned(),
                Ok(false) => "released".to_owned(),
                Err(error) => format!("unknown: {error}"),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = self;
            "unknown: process objects are observed only on Windows".to_owned()
        }
    }

    fn observe(self) -> ProbeChildObservation {
        ProbeChildObservation::Observed {
            pid: self.pid,
            created: self.created,
            at_refusal: self.state(),
        }
    }
}

/// The cold probe's child as the stage lease holds it: its stamp, or why the
/// stamp could not be taken. A failed stamp is kept so that the receipt says
/// why it names no child, instead of recording nothing.
pub(crate) type ProbeChildStamp = std::result::Result<ProbeChild, String>;

/// A probed stage's child as a receipt records it at a refusal: a
/// [`ProbeChild`] and its state then, or the cause its stamp failed with. The
/// latter is stored as `{"error": "<cause>"}`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub(crate) enum ProbeChildObservation {
    Observed {
        pid: u32,
        created: u64,
        at_refusal: String,
    },
    Unstamped {
        error: String,
    },
}

impl ProbeChildObservation {
    fn at_release(stamp: ProbeChildStamp) -> Self {
        match stamp {
            Ok(child) => child.observe(),
            Err(error) => Self::Unstamped { error },
        }
    }
}

/// The receipt as it is stored: paths relative to the version directory, so a
/// moved cache cannot make a receipt name anything outside it.
#[derive(serde::Serialize)]
struct StageCleanupReceipt<'a> {
    version: u32,
    stage: String,
    private: String,
    engine_version: &'a str,
    target: &'a str,
    executable_sha256: &'a str,
    published: bool,
    first_cause: &'a str,
    os_error: Option<i32>,
    attempts: Option<u32>,
    elapsed_ms: Option<u128>,
    descendant: Option<&'a str>,
    probe_child: Option<&'a ProbeChildObservation>,
    recorded_at: u64,
}

/// Record a retained stage so a later open can collect it. `published` says
/// whether its engine was published and verified, or its installation stopped
/// (an error or a cancelled caller) before publication. `probe_child` is the
/// probed engine's child observed at this refusal, when the stage was probed.
///
/// A receipt that cannot be written leaves the stage waiting for an operator;
/// it never turns a published, verified engine into a failed open.
fn record_retained_stage(
    versions: &Path,
    asset: Asset<'_>,
    failure: StageCleanupFailure,
    published: bool,
    probe_child: Option<ProbeChildObservation>,
) -> StageCleanupReport {
    let mut report = StageCleanupReport::new(failure, asset);
    report.published = published;
    report.probe_child = probe_child;
    if let Err(error) = write_stage_receipt(versions, &report) {
        report.receipt_error = Some(format!("{error:#}"));
    }
    report
}

fn write_stage_receipt(versions: &Path, report: &StageCleanupReport) -> Result<()> {
    let stage = relative_to(versions, &report.stage)?;
    ensure!(
        stage.starts_with(".install-") && !stage.contains(['/', '\\']),
        "a retained install stage keeps its private staging name"
    );
    let receipts = versions.join(LEFTOVER_STAGE_RECEIPTS);
    files::ensure_private_directory(&receipts)?;
    let receipt = StageCleanupReceipt {
        version: 1,
        private: relative_to(versions, &report.private)?,
        stage,
        engine_version: report.engine_version,
        target: &report.target,
        executable_sha256: &report.executable_sha256,
        published: report.published,
        first_cause: &report.first_cause,
        os_error: report.os_error,
        attempts: report.attempts,
        elapsed_ms: report.elapsed.map(|elapsed| elapsed.as_millis()),
        descendant: report.descendant.as_deref(),
        probe_child: report.probe_child.as_ref(),
        recorded_at: unix_seconds(),
    };
    let bytes = serde_json::to_vec_pretty(&receipt)?;
    files::write(&receipts.join(format!("{}.json", receipt.stage)), &bytes)
}

/// Emit one structured diagnostics record for a retained install stage.
///
/// Carries exactly what the roadmap ruling requires be reported: the stage
/// path, the first cause with its OS error, the bounded-recovery attempts and
/// elapsed time, and that the engine was published and verified — in one
/// event, collected only by the project's own diagnostics ring
/// (`apps/kuru-tui/src/diagnostics.rs`, which admits exactly this target and
/// these field names, truncating every string field to a bounded length). It
/// is never model-visible and never written to memory: nothing here touches
/// the conversation, a provider request, or a database write. The unabridged
/// receipt (`write_stage_receipt`) remains the durable, on-disk copy of the
/// same facts. A stage whose installation stopped before publication
/// (`published = false`) uses the same fields and its own message.
fn emit_retained_stage_diagnostic(report: &StageCleanupReport) {
    if !report.published {
        tracing::warn!(
            target: "kuru.memory",
            stage = %report.stage.display(),
            digest = %report.executable_sha256,
            published = report.published,
            first_cause = %report.first_cause,
            os_error = report.os_error,
            attempts = report.attempts,
            elapsed_ms = report.elapsed.map(|elapsed| elapsed.as_millis() as u64),
            receipt_error = report.receipt_error.as_deref(),
            "retained private install stage after an unpublished installation"
        );
        return;
    }
    tracing::warn!(
        target: "kuru.memory",
        stage = %report.stage.display(),
        digest = %report.executable_sha256,
        published = report.published,
        first_cause = %report.first_cause,
        os_error = report.os_error,
        attempts = report.attempts,
        elapsed_ms = report.elapsed.map(|elapsed| elapsed.as_millis() as u64),
        receipt_error = report.receipt_error.as_deref(),
        "retained private install stage after published engine"
    );
}

/// Report a sweep that left at least `LEFTOVER_STAGE_CAP` stages behind.
///
/// Reporting only: the cap never decides what to delete. Below it the sweep
/// is silent — an ordinary open says nothing about an empty `.leftovers`.
fn report_leftover_stage_cap(outcome: SweepOutcome) {
    if !outcome.reached_cap {
        return;
    }
    tracing::warn!(
        target: "kuru.memory",
        collected = outcome.collected,
        remaining = outcome.remaining,
        "retained private install stages reached their reporting cap"
    );
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

fn relative_to(versions: &Path, path: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(versions)
        .context("a retained install stage stays inside its engine version directory")?
        .to_string_lossy()
        .into_owned())
}

fn destination_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn verify_existing_cache(
    destination: &Path,
    asset: Asset<'_>,
    ticks: Option<&OpenTicks>,
) -> Result<PathBuf> {
    crate::progress::milestone(ticks, crate::open_timeline::Event::CacheVerifyStart);
    let verified = verified_cache_ticking(destination, asset, ticks.cloned())
        .await
        .with_context(|| {
            format!(
                "Dolt cache is invalid at {}; preserve or remove that version directory and retry",
                destination.display()
            )
        });
    crate::progress::milestone(ticks, crate::open_timeline::Event::CacheVerifyEnd);
    verified
}

#[cfg(all(test, unix))]
async fn verified_cache(directory: &Path, asset: Asset<'_>) -> Result<PathBuf> {
    verified_cache_ticking(directory, asset, None).await
}

/// Verify the published cache, advancing `ticks` per 8 MiB hashed.
async fn verified_cache_ticking(
    directory: &Path,
    asset: Asset<'_>,
    ticks: Option<OpenTicks>,
) -> Result<PathBuf> {
    let binary = directory.join(asset.executable_name);
    let directory_path = directory.to_owned();
    let executable_name = asset.executable_name.to_owned();
    let executable_bytes = asset.executable_bytes;
    let executable_sha256 = asset.executable_sha256.to_owned();
    let license_bytes = asset.license_bytes;
    let license_sha256 = asset.license_sha256.to_owned();
    let notices: Vec<_> = asset
        .notices
        .iter()
        .map(|notice| {
            (
                notice.name.to_owned(),
                notice.bytes,
                notice.sha256.to_owned(),
            )
        })
        .collect();
    // The cache directory is keyed by the pinned engine version, and before
    // activating it the cold path probed a private copy whose full digest
    // matched this executable's pinned digest. A matching full digest here
    // therefore identifies the same bytes that probe ran; a warm open launches
    // no process of its own.
    tokio::task::spawn_blocking(move || {
        CheckedCache::open_and_verify_ticking(
            &directory_path,
            &executable_name,
            executable_bytes,
            &executable_sha256,
            license_bytes,
            &license_sha256,
            &notices,
            &mut ByteTicks::new(ticks.as_ref()),
        )
    })
    .await??;
    Ok(binary)
}

struct CheckedCache {
    directory: Directory,
    executable_name: std::ffi::OsString,
    executable: File,
    licenses: File,
    notices: Vec<(std::ffi::OsString, File)>,
}

impl CheckedCache {
    #[cfg(test)]
    fn open_and_verify(
        path: &Path,
        executable_name: &str,
        executable_bytes: u64,
        executable_sha256: &str,
        license_bytes: u64,
        license_sha256: &str,
        notices: &[(String, u64, String)],
    ) -> Result<Self> {
        Self::open_and_verify_ticking(
            path,
            executable_name,
            executable_bytes,
            executable_sha256,
            license_bytes,
            license_sha256,
            notices,
            &mut ByteTicks::new(None),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the pinned payload inventory plus the open's byte counter"
    )]
    fn open_and_verify_ticking(
        path: &Path,
        executable_name: &str,
        executable_bytes: u64,
        executable_sha256: &str,
        license_bytes: u64,
        license_sha256: &str,
        notices: &[(String, u64, String)],
        hashed: &mut ByteTicks<'_>,
    ) -> Result<Self> {
        let directory = files::directory(path)?;
        let executable_name = std::ffi::OsString::from(executable_name);
        let mut executable = directory.read(&executable_name)?;
        let mut licenses = directory.read(std::ffi::OsStr::new("LICENSES"))?;
        verify_payload_file_ticking(
            &mut executable,
            executable_bytes,
            executable_sha256,
            true,
            &path.join(&executable_name),
            hashed,
        )?;
        verify_payload_file_ticking(
            &mut licenses,
            license_bytes,
            license_sha256,
            false,
            &path.join("LICENSES"),
            hashed,
        )?;
        let notices = notices
            .iter()
            .map(|(name, bytes, sha256)| {
                let name = std::ffi::OsString::from(name);
                let mut file = directory.read(&name)?;
                verify_payload_file_ticking(
                    &mut file,
                    *bytes,
                    sha256,
                    false,
                    &path.join(&name),
                    hashed,
                )?;
                Ok((name, file))
            })
            .collect::<Result<Vec<_>>>()?;
        let checked = Self {
            directory,
            executable_name,
            executable,
            licenses,
            notices,
        };
        checked.revalidate()?;
        Ok(checked)
    }

    fn revalidate(&self) -> Result<()> {
        self.directory.revalidate()?;
        self.directory
            .verify(&self.executable_name, &self.executable)?;
        self.directory
            .verify(std::ffi::OsStr::new("LICENSES"), &self.licenses)?;
        for (name, file) in &self.notices {
            self.directory.verify(name, file)?;
        }
        Ok(())
    }
}

async fn private_probe(binary: PathBuf) -> Result<()> {
    let home = PrivateTemp::new("kuru-dolt-version-", None)?;
    owned_probe(binary, home.path().to_owned(), home).await?;
    Ok(())
}

async fn owned_probe<T: Send + 'static>(binary: PathBuf, home: PathBuf, retained: T) -> Result<T> {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("kuru-dolt-probe".into())
        .spawn(move || {
            // This process owner must survive destruction of its caller's executor.
            // Sending to a canceled caller drops retained resources here, only
            // after the bounded probe and actual owned child reap have completed.
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("create owned Dolt probe executor")
                .and_then(|runtime| runtime.block_on(verify_version(&binary, &home)));
            let _ = send.send((retained, result));
        })
        .context("start owned Dolt probe thread")?;
    let (retained, result) = receive
        .await
        .context("owned Dolt probe did not return its result")?;
    result?;
    Ok(retained)
}

#[derive(Debug)]
struct CheckedColdProbe {
    directory: Directory,
    executable_name: std::ffi::OsString,
    executable: File,
    binary: PathBuf,
    home: PathBuf,
}

impl CheckedColdProbe {
    fn revalidate(&self) -> Result<()> {
        self.directory.revalidate()?;
        self.directory
            .verify(&self.executable_name, &self.executable)?;
        Ok(())
    }

    async fn probe(self, retained: StageLease) -> Result<(Self, StageLease)> {
        let binary = self.binary.clone();
        let home = self.home.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("kuru-dolt-cold-probe".into())
            .spawn(move || {
                // Keep the checked copy, stage and installation authority until
                // the owned process is reaped, including after caller cancellation.
                let mut retained = retained;
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("create owned Dolt cold probe executor")
                    .and_then(|runtime| {
                        self.revalidate().and_then(|()| {
                            runtime.block_on(verify_version_recorded(
                                &binary,
                                &home,
                                VERSION_TIMEOUT,
                                &mut retained.probe_child,
                            ))
                        })
                    });
                let _ = send.send((self, retained, result));
            })
            .context("start owned Dolt cold probe thread")?;
        let (probe, retained, result) = receive
            .await
            .context("owned Dolt cold probe did not return its result")?;
        if let Err(error) = result {
            // Close the probe handles, then resolve the stage through its
            // lease before that lease releases the installation lock.
            drop(probe);
            return Err(retained.discard_after(error));
        }
        Ok((probe, retained))
    }
}

#[cfg(all(test, unix))]
fn prepare_cold_probe(
    candidate: &Path,
    probe_home: &Path,
    asset: Asset<'_>,
) -> Result<CheckedColdProbe> {
    prepare_cold_probe_with(candidate, probe_home, asset, None, |_| Ok(()))
}

#[cfg(test)]
fn prepare_cold_probe_observed(
    candidate: &Path,
    probe_home: &Path,
    asset: Asset<'_>,
    observer: impl FnOnce(&mut File) -> Result<()>,
) -> Result<CheckedColdProbe> {
    prepare_cold_probe_with(candidate, probe_home, asset, None, observer)
}

fn prepare_cold_probe_with(
    candidate: &Path,
    probe_home: &Path,
    asset: Asset<'_>,
    ticks: Option<&OpenTicks>,
    observer: impl FnOnce(&mut File) -> Result<()>,
) -> Result<CheckedColdProbe> {
    let diagnostics = ProbeDiagnostics::new();
    diagnostics.phase("checked-copy-enter");
    let mut bytes = ByteTicks::new(ticks);
    private_directory(probe_home)?;
    let source_directory = files::directory(candidate)?;
    let executable_name = std::ffi::OsStr::new(asset.executable_name);
    let source_path = candidate.join(executable_name);
    let mut source = source_directory.read(executable_name)?;
    verify_payload_file_ticking(
        &mut source,
        asset.executable_bytes,
        asset.executable_sha256,
        true,
        &source_path,
        &mut bytes,
    )?;
    source_directory.verify(executable_name, &source)?;
    diagnostics.phase("source-verified");
    source.rewind()?;

    let probe_directory = files::directory(probe_home)?;
    let probe_path = probe_home.join(executable_name);
    let mut probe_writer = probe_directory.create_new(executable_name)?;
    ensure!(
        std::io::copy(
            &mut (&mut source).take(asset.executable_bytes + 1),
            &mut TickingWriter::new(&mut probe_writer, &mut bytes),
        )? == asset.executable_bytes,
        "Dolt cold probe copy size mismatch"
    );
    diagnostics.phase("copy-written");
    seal_private(&probe_writer, true)?;
    probe_writer.sync_all()?;
    diagnostics.phase("copy-synced");
    observer(&mut probe_writer)?;
    probe_writer.sync_all()?;
    drop(probe_writer);

    // Retain only read authority after the test seam and before execution.
    let mut probe = probe_directory.read(executable_name)?;
    probe.rewind()?;
    verify_payload_file_ticking(
        &mut probe,
        asset.executable_bytes,
        asset.executable_sha256,
        true,
        &probe_path,
        &mut bytes,
    )?;
    source_directory.verify(executable_name, &source)?;
    probe_directory.verify(executable_name, &probe)?;
    ensure!(
        regular_file_info(&source)?.identity != regular_file_info(&probe)?.identity,
        "Dolt cold probe copy unexpectedly retained the source identity"
    );
    diagnostics.phase("copy-verified");
    Ok(CheckedColdProbe {
        directory: probe_directory,
        executable_name: executable_name.to_owned(),
        executable: probe,
        binary: probe_path,
        home: probe_home.to_owned(),
    })
}

#[cfg(test)]
fn extract(archive: &[u8], destination: &Path, asset: Asset<'_>) -> Result<()> {
    extract_ticking(archive, destination, asset, None)
}

/// Extract the pinned archive, advancing `ticks` per 8 MiB of payload
/// written.
fn extract_ticking(
    archive: &[u8],
    destination: &Path,
    asset: Asset<'_>,
    ticks: Option<&OpenTicks>,
) -> Result<()> {
    ensure!(
        asset.compressed_bytes > 0 && asset.compressed_bytes <= MAX_COMPRESSED,
        "pinned Dolt archive exceeds compressed byte budget"
    );
    ensure!(
        archive.len() as u64 == asset.compressed_bytes,
        "bundled Dolt archive size does not match its pin"
    );
    ensure!(
        hex_digest(&Sha256::digest(archive)) == asset.archive_sha256,
        "bundled Dolt archive checksum mismatch; no payload was executed"
    );
    ensure!(
        asset.expanded_bytes <= MAX_EXPANDED,
        "pinned Dolt archive exceeds expanded byte budget"
    );
    let mut written = ByteTicks::new(ticks);
    if asset.format == "zip" {
        return extract_zip(archive, destination, asset, &mut written);
    }
    ensure!(
        asset.format == "tar.gz",
        "unsupported pinned Dolt archive format"
    );
    // Bound decompression before tar sees extension headers or payload lengths.
    let mut expanded = Vec::new();
    let mut decoder = GzDecoder::new(Cursor::new(archive));
    (&mut decoder)
        .take(asset.expanded_bytes + 1)
        .read_to_end(&mut expanded)?;
    ensure!(
        expanded.len() as u64 == asset.expanded_bytes,
        "Dolt expanded archive size does not match its pin"
    );
    ensure!(
        Read::read(&mut decoder.into_inner(), &mut [0_u8; 1])? == 0,
        "Dolt archive contains trailing compressed data"
    );
    private_directory(destination)?;
    let mut archive = tar::Archive::new(Cursor::new(expanded.as_slice()));
    let directory = format!("{}/", asset.stem);
    let bin_directory = format!("{}/bin/", asset.stem);
    let executable = format!("{}/bin/dolt", asset.stem);
    let licenses = format!("{}/LICENSES", asset.stem);
    let mut seen = HashSet::new();
    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        let path = entry.path_bytes().to_vec();
        ensure!(
            seen.insert(path.clone()),
            "Dolt archive contains duplicate entries"
        );
        ensure!(
            entry.header().as_gnu().is_some() && entry.header().link_name_bytes().is_none(),
            "Dolt archive contains unexpected metadata or links"
        );
        let (output, length, checksum, mode) =
            if path == directory.as_bytes() || path == bin_directory.as_bytes() {
                ensure!(
                    entry.header().entry_type().is_dir()
                        && entry.size() == 0
                        && entry.header().mode()? == 0o755,
                    "Dolt archive directory has an unexpected type, size or mode"
                );
                continue;
            } else if path == executable.as_bytes() {
                (
                    "dolt",
                    asset.executable_bytes,
                    asset.executable_sha256,
                    0o755,
                )
            } else if path == licenses.as_bytes() {
                ("LICENSES", asset.license_bytes, asset.license_sha256, 0o644)
            } else {
                bail!("Dolt archive contains an unexpected entry");
            };
        ensure!(
            entry.header().entry_type().is_file()
                && entry.size() == length
                && entry.header().mode()? == mode,
            "Dolt archive payload has an unexpected type, size or mode"
        );
        let path = destination.join(output);
        let mut file = new_private_file(&path)?;
        let copied = std::io::copy(&mut entry, &mut TickingWriter::new(&mut file, &mut written))?;
        ensure!(copied == length, "Dolt archive payload is truncated");
        seal_private(&file, output == "dolt")?;
        file.sync_all()?;
        verify_payload(&path, length, checksum, output == "dolt")?;
    }
    ensure!(
        seen.len() == 4
            && [directory, bin_directory, executable, licenses]
                .iter()
                .all(|name| seen.contains(name.as_bytes())),
        "Dolt archive must contain exactly its four pinned entries"
    );
    let position = archive.into_inner().position() as usize;
    ensure!(
        expanded[position..].iter().all(|byte| *byte == 0),
        "Dolt archive contains trailing tar data"
    );
    #[cfg(unix)]
    File::open(destination)?.sync_all()?;
    Ok(())
}

/// Installation authority over one private install stage: the stage, the
/// cache lock that serializes it, and the engine it installs.
///
/// Every exit resolves the stage before it releases the lock. The stage is
/// removed through `PrivateTemp::close_or_keep` (with Windows' bounded checked
/// recovery); or a refused or uncertain removal is receipted under
/// `.leftovers` and reported to diagnostics; or, after a failed activation,
/// `keep` preserves it as evidence named by the caller's error. Nothing here
/// goes through `tempfile::TempDir::drop`, which discards a failed removal.
///
/// `Drop` resolves the stage of a cancelled or unwinding owner exactly as an
/// unpublished `discard_after` does. It blocks its thread only for that
/// bounded removal (at most `CLEANUP_RETRY_LIMIT` on Windows, one
/// `remove_dir_all` on Unix) and one receipt write - never indefinitely.
struct StageLease {
    staging: Option<PrivateTemp>,
    lock: Option<CacheLock>,
    asset: Asset<'static>,
    /// The cold probe's child, or why it could not be stamped, once the
    /// stage's engine has been probed.
    probe_child: Option<ProbeChildStamp>,
}

impl StageLease {
    fn new(staging: PrivateTemp, lock: CacheLock, asset: Asset<'static>) -> Self {
        Self {
            staging: Some(staging),
            lock: Some(lock),
            asset,
            probe_child: None,
        }
    }

    fn path(&self) -> &Path {
        self.staging
            .as_ref()
            .expect("a live stage lease owns its stage")
            .path()
    }

    /// The engine is published and verified: remove the stage, or receipt and
    /// report it, then release the lock. A retained stage never fails an open.
    fn close_published(mut self) -> Option<StageCleanupReport> {
        self.release(true)
    }

    /// Nothing was published: remove the stage, or receipt and report it,
    /// then release the lock. The caller's error still propagates and names a
    /// retained stage.
    fn discard_after(mut self, error: anyhow::Error) -> anyhow::Error {
        match self.release(false) {
            None => error,
            Some(report) => error.context(format!(
                "retained private install stage at {} after its checked removal failed",
                report.stage.display()
            )),
        }
    }

    /// Preserve the stage as evidence of a failed activation, then release
    /// the lock. The caller names the returned stage in its error.
    fn keep(mut self) -> PathBuf {
        let retained = self
            .staging
            .take()
            .expect("a live stage lease owns its stage")
            .keep();
        drop(self.lock.take());
        retained
    }

    fn release(&mut self, published: bool) -> Option<StageCleanupReport> {
        let report = self.staging.take().and_then(|staging| {
            let mut failure = staging.close_or_keep().err()?;
            failure.cause = failure.cause.context(if published {
                "Dolt engine publication succeeded, but private stage cleanup failed"
            } else {
                "Dolt engine installation stopped before publication, and private stage cleanup failed"
            });
            let versions = failure.stage.parent().unwrap_or(Path::new("")).to_owned();
            // Observed now, at the refusal, while the lock is still held.
            let probe_child = self
                .probe_child
                .take()
                .map(ProbeChildObservation::at_release);
            let report =
                record_retained_stage(&versions, self.asset, failure, published, probe_child);
            emit_retained_stage_diagnostic(&report);
            Some(report)
        });
        // Only now, with its stage gone or receipted and reported, may another
        // installer take the lock.
        drop(self.lock.take());
        report
    }
}

impl Drop for StageLease {
    fn drop(&mut self) {
        self.release(false);
    }
}

struct StagedActivation {
    // Rust drops fields in declaration order. Close the candidate and probe
    // authorities before the lease resolves the stage and releases the lock.
    source: Option<Directory>,
    probe: Option<CheckedColdProbe>,
    lease: StageLease,
}

impl StagedActivation {
    /// The engine is published and verified before this runs, so a stage that
    /// survives its own bounded removal is reported, not raised: only integrity
    /// failures fail an open. The returned report names a retained stage that
    /// the lease already receipted before releasing the lock.
    fn finish_published(self) -> Option<StageCleanupReport> {
        let Self {
            source,
            probe,
            lease,
        } = self;
        drop(source);
        drop(probe);
        lease.close_published()
    }

    fn retain(self, error: anyhow::Error) -> anyhow::Error {
        let Self {
            source,
            probe,
            lease,
        } = self;
        // Close the checked candidate before retaining its stage, then release
        // the cache lock only after stage ownership has been decided.
        drop(source);
        drop(probe);
        let retained = lease.keep();
        error.context(format!(
            "verified Dolt activation failed; preserved private stage at {}",
            retained.display()
        ))
    }
}

#[cfg(test)]
async fn activate_staged(
    staging: PrivateTemp,
    lock: CacheLock,
    candidate: &Path,
    destination: &Path,
) -> Result<Option<StageCleanupReport>> {
    let lease = StageLease::new(staging, lock, BUNDLED_ASSET);
    activate_staged_with(lease, None, candidate, destination, |_| {}).await
}

async fn activate_staged_after_probe(
    lease: StageLease,
    probe: CheckedColdProbe,
    candidate: &Path,
    destination: &Path,
) -> Result<Option<StageCleanupReport>> {
    activate_staged_with(lease, Some(probe), candidate, destination, |_| {}).await
}

#[cfg(test)]
async fn activate_staged_observed(
    staging: PrivateTemp,
    lock: CacheLock,
    candidate: &Path,
    destination: &Path,
    observer: impl FnMut(bool),
) -> Result<Option<StageCleanupReport>> {
    let lease = StageLease::new(staging, lock, BUNDLED_ASSET);
    activate_staged_with(lease, None, candidate, destination, observer).await
}

async fn activate_staged_with(
    lease: StageLease,
    probe: Option<CheckedColdProbe>,
    candidate: &Path,
    destination: &Path,
    mut observer: impl FnMut(bool),
) -> Result<Option<StageCleanupReport>> {
    let mut activation = StagedActivation {
        source: None,
        probe,
        lease,
    };
    activation.source = match files::directory(candidate) {
        Ok(source) => Some(source),
        Err(error) => {
            return Err(activation.retain(error.context("open verified Dolt activation source")));
        }
    };
    let source = activation
        .source
        .as_ref()
        .expect("the activation source is set before any move attempt");

    #[cfg(not(windows))]
    {
        match activate_once(source, destination) {
            Ok(files::DirectoryMove::Moved(_)) => {
                observer(false);
                Ok(activation.finish_published())
            }
            Ok(files::DirectoryMove::ProvenNoMove(error)) => {
                observer(true);
                Err(activation.retain(error))
            }
            Err(error) => Err(activation.retain(error)),
        }
    }

    #[cfg(windows)]
    {
        let deadline = tokio::time::Instant::now() + ACTIVATION_RETRY_LIMIT;
        let mut first_error = None;
        let mut retries = 0_u32;
        loop {
            // A pending retry whose window has closed ends with its first error.
            if let Some(error) = first_error.take_if(|_| tokio::time::Instant::now() >= deadline) {
                return Err(activation.retain(expired_activation_error(error, retries)));
            }
            if first_error.is_some() {
                retries += 1;
            }
            match activate_once(source, destination) {
                Ok(files::DirectoryMove::Moved(_)) => {
                    observer(false);
                    return Ok(activation.finish_published());
                }
                Ok(files::DirectoryMove::ProvenNoMove(error)) => {
                    observer(true);
                    if error
                        .downcast_ref::<kuru_platform::fs::PublicationError>()
                        .is_none_or(|publication| publication.error().raw_os_error() != Some(5))
                    {
                        let error = terminal_activation_error(first_error, error, retries);
                        return Err(activation.retain(error));
                    }
                    let now = tokio::time::Instant::now();
                    if now >= deadline {
                        // A recoverable result that arrives after the window,
                        // even the first one, ends recovery as stopped.
                        let error = match first_error {
                            Some(first_error) => {
                                terminal_activation_error(Some(first_error), error, retries)
                            }
                            None => expired_activation_error(error, retries),
                        };
                        return Err(activation.retain(error));
                    }
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    tokio::time::sleep((deadline - now).min(ACTIVATION_RETRY_SPACING)).await;
                }
                Err(error) => {
                    let error = terminal_activation_error(first_error, error, retries);
                    return Err(activation.retain(error));
                }
            }
        }
    }
}

#[cfg(windows)]
fn terminal_activation_error(
    first_error: Option<anyhow::Error>,
    error: anyhow::Error,
    retries: u32,
) -> anyhow::Error {
    match first_error {
        Some(first_error) => first_error.context(format!(
            "runtime activation recovery stopped after {retries} retries; latest checked result: {error:#}"
        )),
        None => error,
    }
}

#[cfg(windows)]
fn expired_activation_error(error: anyhow::Error, retries: u32) -> anyhow::Error {
    error.context(format!(
        "runtime activation recovery stopped after {retries} retries before starting another native move"
    ))
}

#[cfg(test)]
fn activate(candidate: &Path, destination: &Path) -> Result<()> {
    let source = files::directory(candidate)?;
    match activate_once(&source, destination)? {
        files::DirectoryMove::Moved(_) => Ok(()),
        files::DirectoryMove::ProvenNoMove(error) => Err(error),
    }
}

fn activate_once(source: &Directory, destination: &Path) -> Result<files::DirectoryMove> {
    ensure!(
        fs::symlink_metadata(destination).is_err(),
        "Dolt cache destination appeared during installation"
    );
    match files::move_directory_checked(source, destination) {
        Ok(files::DirectoryMove::Moved(directory)) => Ok(files::DirectoryMove::Moved(directory)),
        Ok(files::DirectoryMove::ProvenNoMove(error)) => Ok(files::DirectoryMove::ProvenNoMove(
            error.context("activate verified Dolt runtime"),
        )),
        Err(error) => Err(error.context("activate verified Dolt runtime")),
    }
}

fn extract_zip(
    bytes: &[u8],
    destination: &Path,
    asset: Asset<'_>,
    written: &mut ByteTicks<'_>,
) -> Result<()> {
    use kuru_archive::zip::{Archive, Limits, MemberKind, MemberSpec};
    let directory = format!("{}/", asset.stem);
    let bin_directory = format!("{}/bin/", asset.stem);
    let executable = format!("{}/bin/{}", asset.stem, asset.executable_name);
    let licenses = format!("{}/LICENSES", asset.stem);
    // Built archives carry their declared third-party notices beside LICENSES;
    // upstream archives declare none and keep exactly four members.
    let notices: Vec<_> = asset
        .notices
        .iter()
        .map(|notice| (format!("{}/{}", asset.stem, notice.name), notice))
        .collect();
    let mut expected = vec![
        MemberSpec {
            name: &directory,
            kind: MemberKind::Directory,
            max_bytes: 0,
            exact_bytes: Some(0),
            unix_mode: Some(0o040755),
        },
        MemberSpec {
            name: &bin_directory,
            kind: MemberKind::Directory,
            max_bytes: 0,
            exact_bytes: Some(0),
            unix_mode: Some(0o040755),
        },
        MemberSpec {
            name: &executable,
            kind: MemberKind::File,
            max_bytes: asset.executable_bytes,
            exact_bytes: Some(asset.executable_bytes),
            unix_mode: Some(0o100755),
        },
        MemberSpec {
            name: &licenses,
            kind: MemberKind::File,
            max_bytes: asset.license_bytes,
            exact_bytes: Some(asset.license_bytes),
            unix_mode: Some(0o100644),
        },
    ];
    expected.extend(notices.iter().map(|(member, notice)| MemberSpec {
        name: member,
        kind: MemberKind::File,
        max_bytes: notice.bytes,
        exact_bytes: Some(notice.bytes),
        unix_mode: Some(0o100644),
    }));
    let mut archive = Archive::open(
        bytes,
        &expected,
        Limits {
            max_compressed_bytes: MAX_COMPRESSED,
            max_expanded_bytes: asset.expanded_bytes,
            allow_ntfs_timestamps: true,
        },
    )?;
    private_directory(destination)?;
    let parent = files::directory(destination)?;
    let payloads = [
        (
            executable.as_str(),
            asset.executable_name,
            asset.executable_bytes,
            asset.executable_sha256,
            true,
        ),
        (
            licenses.as_str(),
            "LICENSES",
            asset.license_bytes,
            asset.license_sha256,
            false,
        ),
    ]
    .into_iter()
    .chain(notices.iter().map(|(member, notice)| {
        (
            member.as_str(),
            notice.name,
            notice.bytes,
            notice.sha256,
            false,
        )
    }));
    for (member, output, size, digest, executable) in payloads {
        let mut file = parent.create_new(std::ffi::OsStr::new(output))?;
        ensure!(
            archive.copy(member, &mut TickingWriter::new(&mut file, written))? == size,
            "Dolt ZIP payload size mismatch"
        );
        file.sync_all()?;
        seal_private(&file, executable)?;
        parent.verify(std::ffi::OsStr::new(output), &file)?;
        drop(file);
        verify_payload(&destination.join(output), size, digest, executable)?;
    }
    #[cfg(unix)]
    File::open(destination)?.sync_all()?;
    Ok(())
}

struct CacheLock {
    file: File,
    _directory: Directory,
}
impl std::ops::Deref for CacheLock {
    type Target = File;
    fn deref(&self) -> &File {
        &self.file
    }
}

struct LockHandle {
    directory: Directory,
    file: File,
    path: PathBuf,
}

fn open_lock_handle(directory: &Path) -> Result<LockHandle> {
    let path = directory.join(".install.lock");
    let directory = files::open_directory(directory, Privacy::OwnerOnly, NameRetention::Pinned)?;
    let file = directory
        .lock_file(files::name(&path)?)
        .context("open stable Dolt installation lock")?;
    checked_regular(&path, false)?;
    Ok(LockHandle {
        directory,
        file,
        path,
    })
}

async fn cache_lock(directory: &Path, timeout: Duration) -> Result<CacheLock> {
    let LockHandle {
        directory,
        file,
        path,
    } = open_lock_handle(directory)?;
    let start = tokio::time::Instant::now();
    loop {
        directory.verify(files::name(&path)?, &file)?;
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => {
                ensure!(
                    start.elapsed() < timeout,
                    "timed out waiting for another Dolt installation; the held lock was preserved"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => return Err(error).context("lock Dolt installation"),
        }
    }
    directory
        .verify(files::name(&path)?, &file)
        .context("Dolt installation lock was replaced while waiting")?;
    Ok(CacheLock {
        file,
        _directory: directory,
    })
}

/// Attempt the exclusive Dolt installation lock without waiting for it.
/// `Ok(None)` means another installer currently holds it - a warm-open
/// caller treats that exactly like an error-free skip, never a failure.
fn try_cache_lock(directory: &Path) -> Result<Option<CacheLock>> {
    let LockHandle {
        directory,
        file,
        path,
    } = open_lock_handle(directory)?;
    directory.verify(files::name(&path)?, &file)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(error) => return Err(error).context("lock Dolt installation"),
    }
    directory
        .verify(files::name(&path)?, &file)
        .context("Dolt installation lock was replaced while waiting")?;
    Ok(Some(CacheLock {
        file,
        _directory: directory,
    }))
}

/// Leftover install stages a sweep could not collect, summed across every
/// receipt it read. Reaching this many changes nothing about removal - the
/// cap only sets `SweepOutcome::reached_cap`, which `report_leftover_stage_cap`
/// turns into one diagnostics record (see `docs/memory.md`). No stage whose
/// removal is rejected or uncertain is ever deleted because of it.
const LEFTOVER_STAGE_CAP: usize = 8;

/// One collection pass over `versions/.leftovers`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SweepOutcome {
    pub collected: usize,
    pub remaining: usize,
    /// `remaining >= LEFTOVER_STAGE_CAP`. Reporting-only: it is read to emit
    /// one diagnostics record, never to decide what to delete.
    pub reached_cap: bool,
}

/// A receipt as read back: only the fields the sweep needs to re-locate and
/// validate its own stage before touching it. Extra fields (the cause, the
/// digest, …) are ignored here, never rejected.
#[derive(serde::Deserialize)]
struct StoredLeftoverStage {
    version: u32,
    stage: String,
}

/// Collect every retained install stage this `versions` directory has a
/// receipt for. Runs only while the caller holds the installation lock -
/// taking `&CacheLock` by reference makes that a type-level fact; this
/// function never acquires or releases it. Only a receipted `.install-*`
/// directory is ever touched: a stage nothing has receipted (including one a
/// concurrent installer is writing right now, since that installer holds the
/// same lock this caller does) is left exactly alone, and a removal that is
/// rejected or uncertain leaves both the stage and its receipt for the next
/// sweep - never a retry loop, never a deletion on uncertainty. Each refusal
/// is recorded in its receipt (`record_sweep_refusal`). Each receipt present
/// when the sweep starts is attempted exactly once: the names are listed
/// before any receipt is rewritten or removed.
///
/// A sweep that leaves no receipt removes the receipts folder itself, but only
/// when nothing else is in it (`remove_empty_leftovers`).
fn sweep_leftover_stages(versions: &Path, _lock: &CacheLock) -> SweepOutcome {
    let mut outcome = SweepOutcome::default();
    let receipts_path = versions.join(LEFTOVER_STAGE_RECEIPTS);
    let Ok(entries) = fs::read_dir(&receipts_path) else {
        return outcome; // no `.leftovers` directory: nothing to sweep
    };
    let Ok(receipts) =
        files::open_directory(&receipts_path, Privacy::OwnerOnly, NameRetention::Movable)
    else {
        return outcome;
    };
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        // Skips e.g. the `staging` directory `files::write` uses.
        .filter(|name| name.ends_with(".json"))
        .collect();
    for name in &names {
        sweep_one_leftover_stage(versions, &receipts, name, &mut outcome);
    }
    outcome.reached_cap = outcome.remaining >= LEFTOVER_STAGE_CAP;
    if outcome.remaining == 0 {
        remove_empty_leftovers(&receipts_path, receipts);
    }
    outcome
}

/// A stored leftover-stage receipt is a few hundred bytes; anything past this
/// generous bound is treated the same as unparsable, never read without a limit.
const LEFTOVER_RECEIPT_LIMIT: u64 = 16 * 1024;

/// Bounded, checked read of one receipt, using the already-open `.leftovers`
/// directory so this never reopens or re-walks its parent. Mirrors
/// `files::read_bytes`'s contract (size limit, then identity `verify`) rather
/// than reading the file directly off the path.
fn read_leftover_receipt(receipts: &Directory, name: &str) -> Result<Vec<u8>> {
    let name = std::ffi::OsStr::new(name);
    let mut file = receipts.read(name)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(LEFTOVER_RECEIPT_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LEFTOVER_RECEIPT_LIMIT,
        "retained-stage receipt exceeds its size limit"
    );
    receipts.verify(name, &file)?;
    Ok(bytes)
}

fn sweep_one_leftover_stage(
    versions: &Path,
    receipts: &Directory,
    name: &str,
    outcome: &mut SweepOutcome,
) {
    let Ok(bytes) = read_leftover_receipt(receipts, name) else {
        // Over the size limit or otherwise unreadable: leave it, report it,
        // never delete a receipt (or its stage) blindly.
        outcome.remaining += 1;
        return;
    };
    let Ok(receipt) = serde_json::from_slice::<StoredLeftoverStage>(&bytes) else {
        outcome.remaining += 1; // unparsable: leave it, never delete blindly
        return;
    };
    if receipt.version != 1
        || !receipt.stage.starts_with(".install-")
        || receipt.stage.contains(['/', '\\'])
    {
        outcome.remaining += 1; // a foreign-shaped receipt: leave it
        return;
    }
    let stage = versions.join(&receipt.stage);
    let refusal = match fs::symlink_metadata(&stage) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // The stage is already gone; only the receipt is left to collect.
            collect_leftover_receipt(receipts, name, outcome);
            return;
        }
        Err(error) => SweepRefusal::other("inspect", &anyhow::Error::from(error)),
        Ok(metadata) if !metadata.is_dir() => SweepRefusal::other(
            "inspect",
            &anyhow::anyhow!("the receipted install stage is not a directory"),
        ),
        Ok(_) => match files::open_directory(&stage, Privacy::OwnerOnly, NameRetention::Movable) {
            Ok(directory) => match directory.remove_tree() {
                Ok(()) => {
                    collect_leftover_receipt(receipts, name, outcome);
                    return;
                }
                // Rejected or Uncertain: leave both.
                Err(error) => SweepRefusal::removal(&error),
            },
            Err(error) => SweepRefusal::other("open", &error),
        },
    };
    outcome.remaining += 1;
    record_sweep_refusal(receipts.path(), name, &bytes, refusal);
}

/// Why one sweep left a receipted stage in place.
struct SweepRefusal {
    /// `Rejected` or `Uncertain` for a checked removal; `open` or `inspect`
    /// when the stage could not be opened or is not a directory.
    phase: String,
    cause: String,
    os_error: Option<i32>,
    /// The refusing entry, relative to the stage.
    descendant: Option<String>,
}

impl SweepRefusal {
    fn removal(error: &kuru_platform::fs::RemovalError) -> Self {
        Self {
            phase: format!("{:?}", error.phase),
            cause: error.to_string(),
            os_error: std::error::Error::source(error)
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .and_then(std::io::Error::raw_os_error),
            descendant: error
                .descendant
                .as_ref()
                .map(|descendant| descendant.to_string_lossy().into_owned()),
        }
    }

    fn other(phase: &str, error: &anyhow::Error) -> Self {
        Self {
            phase: phase.to_owned(),
            cause: format!("{error:#}"),
            os_error: os_error(error),
            descendant: None,
        }
    }
}

/// A receipt's probe child observed at a sweep's refusal: its state when the
/// receipt names a stamped child, `unknown: <cause>` when its stamp failed at
/// the release, and nothing for any other value.
fn probe_child_state_at_sweep(child: &serde_json::Value) -> Option<String> {
    if let Ok(child) = serde_json::from_value::<ProbeChild>(child.clone()) {
        return Some(child.state());
    }
    child
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(|error| format!("unknown: {error}"))
}

/// Record a sweep's refusal in the receipt it read, under the lock the sweep
/// holds: count it in `sweep_refusals` and replace `last_sweep_refusal`, so
/// the receipt stays bounded. Every other field, including any this version
/// does not know, is kept as it was, and `version` stays 1. When the receipt
/// names a probe child, its state is observed now, at this refusal (see
/// [`probe_child_state_at_sweep`]).
///
/// A rewrite that fails leaves the old receipt, which is still valid, and is
/// reported to diagnostics. It never changes what the sweep did.
fn record_sweep_refusal(receipts: &Path, name: &str, bytes: &[u8], refusal: SweepRefusal) {
    let receipt = receipts.join(name);
    let rewritten = (|| -> Result<()> {
        let mut fields: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(bytes)?;
        let probe_child_at_refusal = fields
            .get("probe_child")
            .and_then(probe_child_state_at_sweep);
        let refusals = fields
            .get("sweep_refusals")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .saturating_add(1);
        fields.insert("sweep_refusals".to_owned(), refusals.into());
        fields.insert(
            "last_sweep_refusal".to_owned(),
            serde_json::json!({
                "phase": refusal.phase,
                "cause": refusal.cause,
                "os_error": refusal.os_error,
                "descendant": refusal.descendant,
                "probe_child_at_refusal": probe_child_at_refusal,
                "recorded_at": unix_seconds(),
            }),
        );
        let updated = serde_json::to_vec_pretty(&fields)?;
        ensure!(
            updated.len() as u64 <= LEFTOVER_RECEIPT_LIMIT,
            "the receipt with its refusal would exceed its size limit"
        );
        files::write(&receipt, &updated)
    })();
    if let Err(error) = rewritten {
        tracing::warn!(
            target: "kuru.memory",
            stage = %receipt.display(),
            first_cause = %format!("{error:#}"),
            os_error = os_error(&error),
            "leftover install stage refusal could not be recorded in its receipt"
        );
    }
}

/// Remove the receipts folder once a sweep has left no receipt in it, through
/// one checked removal under the lock the sweep holds. Only an empty folder,
/// or one whose only entry is an empty `staging` directory, is removed:
/// `files::write` keeps a temporary record in `staging` as evidence of an
/// uncertain publication, and nothing else is ever expected there. A refused
/// or uncertain removal leaves the folder for the next sweep and is reported
/// to diagnostics; it is never retried here.
fn remove_empty_leftovers(receipts_path: &Path, receipts: Directory) {
    let only_empty_staging = || -> std::io::Result<bool> {
        for entry in fs::read_dir(receipts_path)? {
            let entry = entry?;
            if entry.file_name() != "staging" || !entry.file_type()?.is_dir() {
                return Ok(false);
            }
            if fs::read_dir(entry.path())?.next().is_some() {
                return Ok(false);
            }
        }
        Ok(true)
    };
    if !only_empty_staging().unwrap_or(false) {
        return;
    }
    if let Err(error) = receipts.remove_tree() {
        let os_error = std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<std::io::Error>())
            .and_then(std::io::Error::raw_os_error);
        tracing::warn!(
            target: "kuru.memory",
            stage = %receipts_path.display(),
            first_cause = %error,
            os_error,
            "leftover install stage receipts folder kept after its removal was refused"
        );
    }
}

/// Count a stage as collected only when its receipt is actually removed too -
/// a stage whose tree is gone (fresh removal or already absent) but whose
/// receipt read or removal fails is left for the next sweep instead, so
/// `collected` never overstates what this pass actually reclaimed.
fn collect_leftover_receipt(receipts: &Directory, name: &str, outcome: &mut SweepOutcome) {
    let name = std::ffi::OsStr::new(name);
    let removed = receipts
        .read(name)
        .ok()
        .and_then(|file| receipts.remove_file(name, file).ok())
        .is_some();
    if removed {
        outcome.collected += 1;
    } else {
        outcome.remaining += 1;
    }
}

/// Best-effort: a leftover-stage sweep never turns a warm open into a failed
/// one. An absent `.leftovers` directory is the ordinary case and costs one
/// metadata call on the calling task, so the common warm open never crosses
/// into a blocking-pool task at all. Only once a receipt might exist does the
/// rest - the directory scan, the non-blocking lock attempt, and the sweep's
/// own removals - run through `spawn_blocking`, exactly like the cold path's
/// extraction and probe work (`provision.rs` above). The task is awaited, so
/// the open returns only after its sweep. A busy lock skips the sweep
/// silently: its holder sweeps when it acquires the lock, or is the receipt's
/// writer. A lock attempt that fails with an error skips it too, and is
/// reported to diagnostics without writing anything.
async fn warm_sweep_if_receipted(cache: PathBuf, versions: PathBuf) {
    let receipts_path = versions.join(LEFTOVER_STAGE_RECEIPTS);
    let is_leftovers_dir =
        matches!(fs::symlink_metadata(&receipts_path), Ok(metadata) if metadata.is_dir());
    if !is_leftovers_dir {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || {
        let receipts_path = versions.join(LEFTOVER_STAGE_RECEIPTS);
        let has_receipt = fs::read_dir(&receipts_path)
            .map(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.ends_with(".json"))
                })
            })
            .unwrap_or(false);
        if !has_receipt {
            return;
        }
        match try_cache_lock(&cache) {
            Ok(Some(lock)) => {
                let outcome = sweep_leftover_stages(&versions, &lock);
                drop(lock);
                report_leftover_stage_cap(outcome);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                target: "kuru.memory",
                stage = %receipts_path.display(),
                first_cause = %format!("{error:#}"),
                os_error = os_error(&error),
                "leftover install stage sweep skipped after the installation lock attempt failed"
            ),
        }
    })
    .await;
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    files::private_dir(path).context("inspect private Dolt directory")
}

fn checked_regular(path: &Path, executable: bool) -> Result<fs::Metadata> {
    let (_parent, file) = files::read(path, Privacy::Inherited)
        .with_context(|| format!("inspect Dolt file {}", path.display()))?;
    let metadata = file.metadata()?;
    #[cfg(unix)]
    ensure!(
        !executable || metadata.permissions().mode() & 0o111 != 0,
        "configured Dolt file is not executable"
    );
    #[cfg(windows)]
    let _ = executable;
    Ok(metadata)
}

fn open_regular(path: &Path) -> Result<File> {
    let (_parent, file) = files::read(path, Privacy::Inherited)?;
    Ok(file)
}

fn new_private_file(path: &Path) -> Result<File> {
    let parent = files::parent(path, Privacy::OwnerOnly, NameRetention::Movable)?;
    Ok(parent.create_new(files::name(path)?)?)
}

fn verify_payload(path: &Path, size: u64, expected: &str, executable: bool) -> Result<()> {
    let (parent, mut file) = files::read(path, Privacy::Inherited)
        .with_context(|| format!("inspect Dolt file {}", path.display()))?;
    verify_payload_file(&mut file, size, expected, executable, path)?;
    parent.verify(files::name(path)?, &file)?;
    Ok(())
}

fn verify_payload_file(
    file: &mut File,
    size: u64,
    expected: &str,
    executable: bool,
    path: &Path,
) -> Result<()> {
    verify_payload_file_ticking(
        file,
        size,
        expected,
        executable,
        path,
        &mut ByteTicks::new(None),
    )
}

/// `verify_payload_file`, advancing `hashed` per 8 MiB hashed.
fn verify_payload_file_ticking(
    file: &mut File,
    size: u64,
    expected: &str,
    executable: bool,
    path: &Path,
    hashed: &mut ByteTicks<'_>,
) -> Result<()> {
    let metadata = file.metadata()?;
    #[cfg(unix)]
    ensure!(
        !executable || metadata.permissions().mode() & 0o111 != 0,
        "configured Dolt file is not executable"
    );
    #[cfg(windows)]
    let _ = executable;
    ensure!(
        metadata.len() == size,
        "Dolt payload size mismatch: {}",
        path.display()
    );
    kuru_platform::fs::require_private(file)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        hashed.add(count);
    }
    ensure!(
        hex_digest(&digest.finalize()) == expected,
        "Dolt payload checksum mismatch: {}",
        path.display()
    );
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Prepare only this owned home; never load the invoking user's Dolt settings.
pub(crate) fn prepare_private_home(home: &Path) -> Result<()> {
    private_directory(home)?;
    for relative in ["home", "tmp", "root", "root/.dolt"] {
        private_directory(&home.join(relative))?;
    }
    let config = home.join("root/.dolt/config_global.json");
    let mut values = if config.try_exists()? {
        let file = open_regular(&config)?;
        ensure!(
            file.metadata()?.len() <= 64 * 1024,
            "private Dolt config is oversized"
        );
        serde_json::from_reader::<_, serde_json::Map<String, serde_json::Value>>(file)
            .context("private Dolt config must be a JSON object")?
    } else {
        ensure!(
            fs::symlink_metadata(&config).is_err(),
            "private Dolt config must not be a dangling link"
        );
        serde_json::Map::new()
    };
    for key in ["metrics.disabled", "versioncheck.disabled"] {
        values.insert(key.to_string(), "true".into());
    }
    files::write_dolt_config(&config, &serde_json::to_vec(&values)?)?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn isolated_command(binary: &Path, home: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.join("home"))
        .env("DOLT_ROOT_PATH", home.join("root"))
        .env("TMPDIR", home.join("tmp"))
        .env("DOLT_DISABLE_EVENT_FLUSH", "1")
        .current_dir(home)
        .kill_on_drop(true);
    command
}

/// Execute a finite, isolated exact-version probe after configuring its private
/// home to suppress upstream version checks and metrics child processes.
pub async fn verify_version(binary: &Path, private_home: &Path) -> Result<()> {
    verify_version_with_timeout(binary, private_home, VERSION_TIMEOUT).await
}

async fn verify_version_with_timeout(binary: &Path, home: &Path, timeout: Duration) -> Result<()> {
    verify_version_recorded(binary, home, timeout, &mut None).await
}

/// [`verify_version_with_timeout`], recording the probe's child process in
/// `probe_child` once it has started (Windows only), or the cause its stamp
/// failed with. The child itself, and so every handle of ours to it, is
/// dropped before this returns.
async fn verify_version_recorded(
    binary: &Path,
    home: &Path,
    timeout: Duration,
    probe_child: &mut Option<ProbeChildStamp>,
) -> Result<()> {
    let diagnostics = ProbeDiagnostics::new();
    diagnostics.phase("private-home-enter");
    checked_regular(binary, true)?;
    prepare_private_home(home)?;
    diagnostics.phase("private-home-ready");
    diagnostics.phase("native-create-enter");
    let mut child = crate::engine::spawn(
        binary,
        home,
        home,
        vec!["version".into()],
        Vec::new(),
        false,
    )
    .await
    .context("start configured Dolt executable")?;
    diagnostics.phase("native-create-returned");
    #[cfg(windows)]
    {
        *probe_child = Some(
            child
                .stamp()
                .map(|stamp| ProbeChild {
                    pid: stamp.id,
                    created: stamp.created,
                })
                .map_err(|error| format!("stamp the probe child: {error}")),
        );
    }
    #[cfg(not(windows))]
    let _ = probe_child;
    let stdout = child.stdout().context("Dolt version stdout is missing")?;
    let stderr = child.stderr().context("Dolt version stderr is missing")?;
    let stdout_facts = PipeFacts::default();
    let stderr_facts = PipeFacts::default();
    diagnostics.phase("wait-enter");
    let result = tokio::time::timeout(timeout, async {
        let (status, stdout, _stderr) = tokio::try_join!(
            async { Ok::<_, anyhow::Error>(child.wait().await?) },
            bounded_output(stdout, &stdout_facts),
            bounded_output(stderr, &stderr_facts),
        )?;
        ensure!(status.success(), "Dolt version probe failed");
        ensure!(
            std::str::from_utf8(&stdout)?.trim() == format!("dolt version {DOLT_VERSION}"),
            "Kuru requires full Dolt {DOLT_VERSION}; configured executable reports another version"
        );
        Ok(())
    })
    .await;
    let timed_out = result.is_err();
    let result = result.unwrap_or_else(|_| Err(anyhow::anyhow!("Dolt version probe timed out")));
    if result.is_err() {
        diagnostics.refusal(&child, &stdout_facts, &stderr_facts, timed_out);
        diagnostics.phase("cleanup-enter");
        let _ = child.kill();
        if child.wait().await.is_err() {
            eprintln!(
                "Dolt version cleanup is delayed; retaining the owned process and probe resources"
            );
            loop {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        diagnostics.phase("cleanup-reaped");
    } else {
        diagnostics.phase("verified-reaped");
    }
    result
}

async fn bounded_output(mut reader: impl AsyncRead + Unpin, facts: &PipeFacts) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let remaining = ((OUTPUT_LIMIT + 1) as usize - bytes.len()).min(buffer.len());
        let read = reader.read(&mut buffer[..remaining]).await?;
        facts.read(read);
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..read]);
        ensure!(
            bytes.len() as u64 <= OUTPUT_LIMIT,
            "Dolt version output exceeded its limit"
        );
    }
}

#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod sweep_tests;
#[cfg(all(test, unix))]
mod tests;
