//! Leftover install stage collection, on every OS.
//!
//! These tests build receipted `.install-*` stages directly and drive
//! `sweep_leftover_stages` and `warm_sweep_if_receipted` themselves, so they
//! need neither the extractor seam nor an executed probe. A refusal is an
//! event each test controls: a nested symlink on Unix, a nested file held
//! without `FILE_SHARE_DELETE` on Windows. Releasing it is the test's own
//! step, never a wait.

use super::native_tests::{gated_cache_lock, observe_leftover_records};
use super::*;

/// The message of the record a refused `.leftovers` removal emits.
const KEPT_RECORD: &str =
    "leftover install stage receipts folder kept after its removal was refused";
/// The message of the record a failed warm lock attempt emits.
const SKIPPED_RECORD: &str =
    "leftover install stage sweep skipped after the installation lock attempt failed";

struct Fixture {
    _root: crate::test_support::TempDir,
    cache: PathBuf,
    versions: PathBuf,
}

fn fixture() -> Fixture {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache");
    private_directory(&cache).unwrap();
    #[cfg(unix)]
    let cache = cache.canonicalize().unwrap();
    let versions = cache.join("9.9.9-fixture");
    private_directory(&versions).unwrap();
    Fixture {
        _root: root,
        cache,
        versions,
    }
}

/// An owner-private `.install-*` stage under `versions` and its receipt, as
/// `StageLease::release` writes one for a published engine.
fn receipted_stage(versions: &Path, name: &str) -> PathBuf {
    let stage = versions.join(name);
    private_directory(&stage).unwrap();
    write_receipt(versions, &stage);
    stage
}

fn write_receipt(versions: &Path, stage: &Path) {
    let failure = crate::files::StageCleanupFailure {
        stage: stage.to_owned(),
        private: stage.join("private"),
        cause: anyhow::Error::msg("fixture cleanup failure"),
    };
    write_stage_receipt(versions, &StageCleanupReport::new(failure, BUNDLED_ASSET)).unwrap();
}

fn receipts_path(versions: &Path) -> PathBuf {
    versions.join(LEFTOVER_STAGE_RECEIPTS)
}

fn receipt_files(versions: &Path) -> Vec<PathBuf> {
    fs::read_dir(receipts_path(versions))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect()
}

fn receipt_of(versions: &Path, stage: &Path) -> PathBuf {
    receipts_path(versions).join(format!(
        "{}.json",
        stage.file_name().unwrap().to_string_lossy()
    ))
}

fn read_json(path: &Path) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

async fn sweep(fixture: &Fixture) -> SweepOutcome {
    let lock = gated_cache_lock(&fixture.cache, Duration::from_secs(1))
        .await
        .unwrap();
    let outcome = sweep_leftover_stages(&fixture.versions, &lock);
    drop(lock);
    outcome
}

/// A warm-open sweep. Its non-blocking lock attempt runs under the spawn gate,
/// like every real lock acquisition in this binary (see `crate::spawn_gate`).
async fn warm_sweep(fixture: &Fixture) {
    let _gate = crate::spawn_gate::locking_async().await;
    warm_sweep_if_receipted(fixture.cache.clone(), fixture.versions.clone()).await;
}

/// Refuses the checked removal of the stage it was placed in, through the
/// only entry of the stage's only subdirectory, until it is released.
struct RemovalBlocker {
    /// The refusing entry relative to the stage.
    descendant: PathBuf,
    #[cfg(unix)]
    link: PathBuf,
    #[cfg(windows)]
    _held: File,
}

impl RemovalBlocker {
    /// A nested symlink, which checked removal never follows or unlinks.
    #[cfg(unix)]
    fn place(stage: &Path) -> Self {
        let nested = stage.join("sub");
        private_directory(&nested).unwrap();
        let outside = stage.parent().unwrap().join("outside-target");
        private_directory(&outside).unwrap();
        let link = nested.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        Self {
            descendant: Path::new("sub").join("link"),
            link,
        }
    }

    /// A nested file opened without `FILE_SHARE_DELETE`, as a loaded image or
    /// a foreign scanner would hold it.
    #[cfg(windows)]
    fn place(stage: &Path) -> Self {
        use std::io::Write;
        use std::os::windows::fs::OpenOptionsExt;

        let nested = stage.join("sub");
        private_directory(&nested).unwrap();
        let directory = files::directory(&nested).unwrap();
        directory
            .create_new(std::ffi::OsStr::new("held"))
            .unwrap()
            .write_all(b"fixture-only held entry")
            .unwrap();
        drop(directory);
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0x3)
            .open(nested.join("held"))
            .unwrap();
        Self {
            descendant: Path::new("sub").join("held"),
            _held: held,
        }
    }

    fn release(self) {
        #[cfg(unix)]
        fs::remove_file(&self.link).unwrap();
    }
}

fn stage_identity(stage: &Path) -> kuru_platform::fs::FileIdentity {
    files::directory(stage).unwrap().identity()
}

#[tokio::test]
async fn sweep_collects_only_receipted_stages() {
    let fixture = fixture();
    let receipted = receipted_stage(&fixture.versions, ".install-receipted");
    private_directory(&receipted.join("private")).unwrap();
    let orphan = fixture.versions.join(".install-orphan");
    private_directory(&orphan).unwrap();

    let outcome = sweep(&fixture).await;

    assert_eq!(outcome.collected, 1);
    assert_eq!(outcome.remaining, 0);
    assert!(!receipted.exists(), "a receipted stage is collected");
    assert!(
        orphan.is_dir(),
        "an unreceipted leftover stage is never swept"
    );
    assert!(receipt_files(&fixture.versions).is_empty());
}

#[test]
fn collect_leftover_receipt_never_counts_a_receipt_it_could_not_remove() {
    let fixture = fixture();
    let receipts_path = receipts_path(&fixture.versions);
    files::ensure_private_directory(&receipts_path).unwrap();
    let receipts =
        files::open_directory(&receipts_path, Privacy::OwnerOnly, NameRetention::Movable).unwrap();

    let mut outcome = SweepOutcome::default();
    // No receipt was ever written at this name: the read must fail, and the
    // stage it would have named must not be miscounted as collected.
    collect_leftover_receipt(&receipts, "missing-stage.json", &mut outcome);

    assert_eq!(
        outcome.collected, 0,
        "a receipt that could not be read must never count as collected"
    );
    assert_eq!(outcome.remaining, 1);
}

#[tokio::test]
async fn sweep_leaves_a_stage_and_its_receipt_when_removal_is_rejected() {
    let fixture = fixture();
    let stage = fixture.versions.join(".install-blocked");
    private_directory(&stage).unwrap();
    let blocker = RemovalBlocker::place(&stage);
    write_receipt(&fixture.versions, &stage);

    let outcome = sweep(&fixture).await;

    assert_eq!(outcome.collected, 0);
    assert_eq!(outcome.remaining, 1);
    assert!(
        stage.join(&blocker.descendant).symlink_metadata().is_ok(),
        "a rejected removal leaves the stage untouched"
    );
    assert_eq!(
        receipt_files(&fixture.versions).len(),
        1,
        "a rejected removal leaves the receipt for a later attempt"
    );
    blocker.release();
}

#[tokio::test]
async fn sweep_reports_the_cap_without_deleting_any_uncollectable_stage() {
    let fixture = fixture();
    let mut blocked = Vec::new();
    for index in 0..LEFTOVER_STAGE_CAP {
        let stage = fixture.versions.join(format!(".install-blocked-{index}"));
        // A plain file where a directory is expected: the sweep's own guard
        // must leave it, never a permission trick or a real removal race.
        fs::write(&stage, b"not a directory").unwrap();
        write_receipt(&fixture.versions, &stage);
        blocked.push(stage);
    }

    let outcome = sweep(&fixture).await;

    assert_eq!(outcome.collected, 0);
    assert!(
        outcome.remaining >= LEFTOVER_STAGE_CAP,
        "the cap is reached: {}",
        outcome.remaining
    );
    assert!(outcome.reached_cap);
    for stage in &blocked {
        assert!(
            stage.is_file(),
            "reaching the cap never deletes an uncollectable stage"
        );
    }
    assert_eq!(
        receipt_files(&fixture.versions).len(),
        LEFTOVER_STAGE_CAP,
        "reaching the cap never drops a receipt"
    );
}

#[tokio::test]
async fn sweep_leaves_an_oversized_or_unparsable_receipt_as_remaining() {
    let fixture = fixture();
    // Both stages exist and are otherwise perfectly collectible; only their
    // receipts are bad, so a correct sweep must never reach the stage removal
    // branch for either one.
    let oversized_stage = fixture.versions.join(".install-oversized");
    private_directory(&oversized_stage).unwrap();
    let malformed_stage = fixture.versions.join(".install-malformed");
    private_directory(&malformed_stage).unwrap();

    let receipts_path = receipts_path(&fixture.versions);
    files::ensure_private_directory(&receipts_path).unwrap();
    // One byte past `LEFTOVER_RECEIPT_LIMIT`: `read_leftover_receipt`'s bounded
    // `take` must reject it rather than silently truncating and parsing a
    // partial document.
    let oversized_bytes = vec![b'a'; (LEFTOVER_RECEIPT_LIMIT + 1) as usize];
    files::write(
        &receipts_path.join(".install-oversized.json"),
        &oversized_bytes,
    )
    .unwrap();
    // Under the limit but not valid JSON at all.
    files::write(&receipts_path.join(".install-malformed.json"), b"not json").unwrap();

    let outcome = sweep(&fixture).await;

    assert_eq!(outcome.collected, 0);
    assert_eq!(outcome.remaining, 2);
    assert!(
        oversized_stage.is_dir(),
        "an over-limit receipt leaves its stage untouched"
    );
    assert!(
        malformed_stage.is_dir(),
        "an unparsable receipt leaves its stage untouched"
    );
    assert_eq!(
        receipt_files(&fixture.versions).len(),
        2,
        "neither a too-large nor an unparsable receipt is ever deleted"
    );
    assert_eq!(
        fs::read(receipts_path.join(".install-oversized.json")).unwrap(),
        oversized_bytes,
        "an unreadable receipt is never rewritten"
    );
}

#[tokio::test]
async fn sweep_refusal_rewrites_only_the_refusal_fields() {
    let fixture = fixture();
    let stage = fixture.versions.join(".install-held");
    private_directory(&stage).unwrap();
    let blocker = RemovalBlocker::place(&stage);
    write_receipt(&fixture.versions, &stage);
    let identity = stage_identity(&stage);
    let receipt = receipt_of(&fixture.versions, &stage);
    let original = read_json(&receipt);
    assert!(!original.contains_key("sweep_refusals"));

    for expected in 1..=2_u64 {
        let outcome = sweep(&fixture).await;
        assert_eq!(
            outcome,
            SweepOutcome {
                collected: 0,
                remaining: 1,
                reached_cap: false,
            }
        );
        assert_eq!(
            stage_identity(&stage),
            identity,
            "a refused sweep keeps the original stage"
        );
        let bytes = fs::read(&receipt).unwrap();
        assert!(bytes.len() as u64 <= LEFTOVER_RECEIPT_LIMIT);
        let recorded = read_json(&receipt);
        for (field, value) in &original {
            assert_eq!(
                recorded.get(field),
                Some(value),
                "a refusal keeps the original `{field}`"
            );
        }
        assert_eq!(recorded["sweep_refusals"], expected, "{recorded:?}");
        let last = &recorded["last_sweep_refusal"];
        assert_eq!(last["phase"], "Rejected", "{last}");
        assert!(
            last["cause"]
                .as_str()
                .is_some_and(|cause| cause.contains("removal at")),
            "{last}"
        );
        assert_eq!(
            last["descendant"],
            blocker.descendant.to_string_lossy().as_ref(),
            "the refusal names the refusing entry relative to the stage: {last}"
        );
        #[cfg(windows)]
        assert_eq!(last["os_error"], 32, "{last}");
        #[cfg(unix)]
        assert!(last["os_error"].is_i64(), "{last}");
        assert!(
            last["probe_child_at_refusal"].is_null(),
            "a receipt without a probe child records none: {last}"
        );
        assert!(last["recorded_at"].is_u64(), "{last}");
    }

    blocker.release();
    let outcome = sweep(&fixture).await;
    assert_eq!(outcome.collected, 1);
    assert_eq!(outcome.remaining, 0);
    assert!(!stage.exists());
    assert!(!receipt.exists());
    assert!(
        !receipts_path(&fixture.versions).exists(),
        "collecting the last receipt removes the receipts folder"
    );
}

#[tokio::test]
async fn sweep_that_collects_the_last_receipt_removes_leftovers() {
    let fixture = fixture();
    let first = receipted_stage(&fixture.versions, ".install-first");
    let second = receipted_stage(&fixture.versions, ".install-second");

    let outcome = sweep(&fixture).await;

    assert_eq!(outcome.collected, 2);
    assert!(!first.exists() && !second.exists());
    assert!(
        !receipts_path(&fixture.versions).exists(),
        "a sweep that collects the last receipt removes the receipts folder and its empty staging"
    );
}

#[tokio::test]
async fn sweep_keeps_leftovers_while_a_receipt_remains() {
    let fixture = fixture();
    let collected = receipted_stage(&fixture.versions, ".install-free");
    let stage = fixture.versions.join(".install-held");
    private_directory(&stage).unwrap();
    let blocker = RemovalBlocker::place(&stage);
    write_receipt(&fixture.versions, &stage);

    let outcome = sweep(&fixture).await;

    assert_eq!((outcome.collected, outcome.remaining), (1, 1));
    assert!(!collected.exists());
    assert_eq!(receipt_files(&fixture.versions).len(), 1);
    blocker.release();
}

#[tokio::test]
async fn sweep_keeps_leftovers_whose_staging_holds_a_record() {
    let fixture = fixture();
    let stage = receipted_stage(&fixture.versions, ".install-free");
    // An uncertain receipt publication keeps its temporary record in
    // `staging` as evidence; nothing may remove it.
    let record = receipts_path(&fixture.versions)
        .join("staging")
        .join("record-fixture.tmp");
    files::write(&record, b"an uncertain receipt publication").unwrap();

    let outcome = sweep(&fixture).await;

    assert_eq!(outcome.collected, 1);
    assert!(!stage.exists());
    assert_eq!(
        fs::read(&record).unwrap(),
        b"an uncertain receipt publication"
    );
}

#[tokio::test]
async fn a_refused_leftovers_removal_is_reported() {
    let fixture = fixture();
    let receipts = receipts_path(&fixture.versions);
    files::ensure_private_directory(&receipts.join("staging")).unwrap();
    let (observer, observed) = observe_leftover_records(&fixture.cache);
    #[cfg(unix)]
    let restore = {
        use std::os::unix::fs::PermissionsExt;
        // Without write permission on the version directory the receipts
        // folder cannot be unlinked from it (CI runs as an ordinary user).
        fs::set_permissions(&fixture.versions, fs::Permissions::from_mode(0o500)).unwrap();
        || {
            fs::set_permissions(&fixture.versions, fs::Permissions::from_mode(0o700)).unwrap();
        }
    };
    #[cfg(windows)]
    let held = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0x3)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(receipts.join("staging"))
            .unwrap()
    };

    let outcome = sweep(&fixture).await;
    #[cfg(unix)]
    restore();
    #[cfg(windows)]
    drop(held);
    drop(observer);

    assert_eq!(outcome, SweepOutcome::default());
    assert!(receipts.is_dir(), "a refused removal leaves the folder");
    let observed = std::mem::take(&mut *observed.lock().unwrap());
    assert_eq!(observed.len(), 1, "{observed:?}");
    assert_eq!(observed[0].message, KEPT_RECORD, "{observed:?}");
    assert_eq!(observed[0].stage, receipts, "{observed:?}");
    assert!(!observed[0].first_cause.is_empty(), "{observed:?}");

    // Nothing holds it any more: the next sweep removes it.
    assert_eq!(sweep(&fixture).await, SweepOutcome::default());
    assert!(!receipts.exists());
}

#[tokio::test]
async fn warm_sweep_collects_a_receipt_when_the_lock_is_free() {
    let fixture = fixture();
    let stage = receipted_stage(&fixture.versions, ".install-free");

    warm_sweep(&fixture).await;

    assert!(!stage.exists());
    assert!(receipt_files(&fixture.versions).is_empty());
    assert!(
        !receipts_path(&fixture.versions).exists(),
        "a warm sweep that collects the last receipt removes the receipts folder"
    );
}

#[tokio::test]
async fn warm_sweep_leaves_a_receipt_untouched_when_the_lock_is_busy() {
    let fixture = fixture();
    let stage = receipted_stage(&fixture.versions, ".install-free");
    let receipt = receipt_of(&fixture.versions, &stage);
    let before = fs::read(&receipt).unwrap();
    let lock = gated_cache_lock(&fixture.cache, Duration::from_secs(1))
        .await
        .unwrap();

    tokio::time::timeout(
        Duration::from_secs(30),
        warm_sweep_if_receipted(fixture.cache.clone(), fixture.versions.clone()),
    )
    .await
    .expect("a warm sweep never waits for a busy installation lock");

    assert!(stage.is_dir());
    assert_eq!(
        fs::read(&receipt).unwrap(),
        before,
        "a skipped sweep records no refusal"
    );
    // The holder sweeps before its own work, as every acquisition does.
    let outcome = sweep_leftover_stages(&fixture.versions, &lock);
    drop(lock);
    assert_eq!(outcome.collected, 1);
    assert!(!stage.exists() && !receipt.exists());
}

#[tokio::test]
async fn warm_sweep_reports_a_failed_lock_attempt() {
    let fixture = fixture();
    let stage = receipted_stage(&fixture.versions, ".install-free");
    let receipt = receipt_of(&fixture.versions, &stage);
    let before = fs::read(&receipt).unwrap();
    // A directory where the installation lock file belongs: the lock attempt
    // fails with an error, which is not a busy lock.
    private_directory(&fixture.cache.join(".install.lock")).unwrap();
    let (observer, observed) = observe_leftover_records(&fixture.cache);

    warm_sweep(&fixture).await;
    drop(observer);

    let observed = std::mem::take(&mut *observed.lock().unwrap());
    assert_eq!(observed.len(), 1, "{observed:?}");
    assert_eq!(observed[0].message, SKIPPED_RECORD, "{observed:?}");
    assert_eq!(
        observed[0].stage,
        receipts_path(&fixture.versions),
        "{observed:?}"
    );
    assert!(!observed[0].first_cause.is_empty(), "{observed:?}");
    assert!(stage.is_dir());
    assert_eq!(fs::read(&receipt).unwrap(), before, "nothing is written");
}

#[test]
fn a_stage_cleanup_report_names_the_refusing_descendant() {
    let fixture = fixture();
    let stage = fixture.versions.join(".install-named");
    private_directory(&stage).unwrap();
    let blocker = RemovalBlocker::place(&stage);
    let error = files::directory(&stage).unwrap().remove_tree().unwrap_err();
    let failure = crate::files::StageCleanupFailure {
        stage: stage.clone(),
        private: stage.join("private"),
        cause: anyhow::Error::new(error).context("fixture checked stage removal"),
    };

    write_stage_receipt(
        &fixture.versions,
        &StageCleanupReport::new(failure, BUNDLED_ASSET),
    )
    .unwrap();

    let receipt = read_json(&receipt_of(&fixture.versions, &stage));
    assert_eq!(
        receipt["descendant"],
        blocker.descendant.to_string_lossy().as_ref(),
        "the receipt names the refusing entry relative to the stage: {receipt:?}"
    );
    assert!(receipt["probe_child"].is_null(), "{receipt:?}");
    blocker.release();
}
