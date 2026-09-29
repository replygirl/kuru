use super::*;
use kuru_archive::zip::{Limits, MemberKind, WriteMember, write};
use kuru_platform::fs::regular_file_info;
#[cfg(unix)]
use std::io::{Seek, SeekFrom, Write};
// Not windows-gated: `wait_for_fixture_binary_absence` below is exercised by
// unix-runnable unit tests even though its only production caller is
// windows-only, since this whole module is already `#[cfg(test)]`.
use std::{io, thread, time::Instant};

const EXE: &[u8] = b"MZ fixture bytes, deliberately never executed";
const NOTICES: &[u8] = b"exact upstream notice fixture";

/// [`cache_lock`], serialised against this lib's own spawning fixtures; see
/// `crate::spawn_gate`. A single choke point so every real-cache-lock test
/// below is covered without gating each call site by hand.
async fn gated_cache_lock(directory: &Path, timeout: Duration) -> Result<CacheLock> {
    let _gate = crate::spawn_gate::locking_async().await;
    cache_lock(directory, timeout).await
}

/// [`verify_version`], serialised against this lib's own advisory-lock
/// tests; see `crate::spawn_gate`. Only Windows tests below call this today.
#[cfg(windows)]
async fn gated_verify_version(binary: &Path, private_home: &Path) -> Result<()> {
    let _gate = crate::spawn_gate::spawning().await;
    verify_version(binary, private_home).await
}

/// What was true at the instant a retained-stage diagnostic was emitted.
#[derive(Debug)]
pub(super) struct RetainedStageObservation {
    /// The event's full message.
    pub message: String,
    /// The event's `published` field.
    pub published: Option<bool>,
    /// The event's `stage` field.
    pub stage: PathBuf,
    /// A fresh handle could not take the installation lock.
    pub lock_held: bool,
    /// The stage's `.leftovers` receipt already existed.
    pub receipted: bool,
}

/// The fields a retained-stage observer reads from the `kuru.memory` event.
#[derive(Default)]
struct RetainedStageFields {
    message: String,
    stage: String,
    published: Option<bool>,
}

impl tracing::field::Visit for RetainedStageFields {
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if field.name() == "published" {
            self.published = Some(value);
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "stage" => self.stage = format!("{value:?}"),
            _ => {}
        }
    }
}

type RetainedStageObserverFn = std::sync::Arc<dyn Fn(&RetainedStageFields) + Send + Sync>;

struct RetainedStageObserverEntry {
    id: u64,
    scope: PathBuf,
    observer: RetainedStageObserverFn,
}

/// Observers of retained-stage events, scoped by the stage path.
static RETAINED_STAGE_OBSERVERS: std::sync::Mutex<Vec<RetainedStageObserverEntry>> =
    std::sync::Mutex::new(Vec::new());

/// The process-wide recorder of `kuru.memory` retained-stage events.
///
/// It is installed once as the global default dispatcher, so every thread
/// resolves to it: a report is observed on whichever thread resolves the
/// lease (a cancelled task's drop, a blocking worker or the caller). A
/// thread-scoped subscriber cannot do this. tracing-core caches callsite
/// interest process-wide, and while one scoped subscriber is the only
/// dispatcher, a callsite first reached on a thread without it is cached as
/// never enabled, hiding the event from the scoped subscriber too (CI run
/// 36424722859). No other code in this test binary installs a subscriber.
struct RetainedStageRecorder;

impl tracing::Subscriber for RetainedStageRecorder {
    fn register_callsite(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        if metadata.target() == "kuru.memory" {
            tracing::subscriber::Interest::always()
        } else {
            tracing::subscriber::Interest::never()
        }
    }

    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "kuru.memory"
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        if event.metadata().target() != "kuru.memory" {
            return;
        }
        let mut fields = RetainedStageFields::default();
        event.record(&mut fields);
        if !fields.message.starts_with("retained private install stage") {
            return;
        }
        // Run the observers outside the registry lock: they inspect the
        // filesystem and the installation lock.
        let stage = Path::new(&fields.stage);
        let observers: Vec<RetainedStageObserverFn> = RETAINED_STAGE_OBSERVERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|entry| stage.starts_with(&entry.scope))
            .map(|entry| std::sync::Arc::clone(&entry.observer))
            .collect();
        for observer in observers {
            observer(&fields);
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Install [`RetainedStageRecorder`] as the process's global dispatcher once,
/// then rebuild every cached callsite interest against it.
///
/// `Dispatch::new` registers the recorder before the global default is
/// stored, so a callsite that another thread registered in that gap can have
/// cached `never` from the then-empty default. tracing-core 0.1.36 links a
/// callsite into its registry before computing its interest, so the rebuild
/// here (repeated on every observer registration) reaches it. The one
/// remaining interleaving, a registration that read the empty default before
/// the store and wrote its interest after this rebuild, fails loudly as a
/// missing observation; it cannot pass falsely.
fn install_retained_stage_recorder() {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        #[expect(
            clippy::disallowed_methods,
            reason = "this is the process-wide recorder the ban points to: installed once per test process behind a OnceLock, then every cached callsite interest is rebuilt"
        )]
        tracing::subscriber::set_global_default(RetainedStageRecorder).expect(
            "the retained-stage recorder is the only global tracing subscriber in kuru-memory's tests",
        );
    });
    tracing::callsite::rebuild_interest_cache();
}

/// Unregisters its observer when dropped.
pub(super) struct RetainedStageObserver {
    id: u64,
}

impl Drop for RetainedStageObserver {
    fn drop(&mut self) {
        RETAINED_STAGE_OBSERVERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|entry| entry.id != self.id);
    }
}

/// Observe every `kuru.memory` retained-stage diagnostic whose `stage` lies
/// under `scope`, recording the event's message and fields together with the
/// installation lock and receipt state at the instant it is emitted.
///
/// The observation is the product's own tracing event, delivered by the
/// process-wide [`RetainedStageRecorder`]; the registry is scoped by path, so
/// concurrent tests with separate directories cannot see or hide each other's
/// events.
pub(super) fn observe_retained_stage_reports(
    scope: &Path,
    lock_path: &Path,
) -> (
    RetainedStageObserver,
    std::sync::Arc<std::sync::Mutex<Vec<RetainedStageObservation>>>,
) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    install_retained_stage_recorder();
    let observed = std::sync::Arc::<std::sync::Mutex<Vec<_>>>::default();
    let sink = std::sync::Arc::clone(&observed);
    let lock_path = lock_path.to_owned();
    let observer = move |fields: &RetainedStageFields| {
        let stage = PathBuf::from(&fields.stage);
        let receipted = match (stage.parent(), stage.file_name()) {
            (Some(versions), Some(name)) => versions
                .join(LEFTOVER_STAGE_RECEIPTS)
                .join(format!("{}.json", name.to_string_lossy()))
                .is_file(),
            _ => false,
        };
        let contender = open_regular(&lock_path).unwrap();
        let lock_held = match contender.try_lock() {
            Ok(()) => false,
            Err(TryLockError::WouldBlock) => true,
            Err(error) => panic!("inspect the installation lock: {error}"),
        };
        drop(contender);
        sink.lock().unwrap().push(RetainedStageObservation {
            message: fields.message.clone(),
            published: fields.published,
            stage,
            lock_held,
            receipted,
        });
    };
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    RETAINED_STAGE_OBSERVERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(RetainedStageObserverEntry {
            id,
            scope: scope.to_owned(),
            observer: std::sync::Arc::new(observer),
        });
    (RetainedStageObserver { id }, observed)
}

/// The `.json` receipts in a `.leftovers` directory. `files::write` keeps its
/// own `staging` directory beside them, which is not a receipt; the product's
/// sweep skips it the same way.
#[cfg(windows)]
fn leftover_stage_receipts(receipts: &Path) -> Vec<PathBuf> {
    fs::read_dir(receipts)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect()
}

/// Bound both the removal retry below and the post-removal absence wait: the
/// fixture deletes a cache binary the warm probes just executed, so Windows
/// may refuse the delete-capable open, refuse the disposition itself, or
/// leave the name delete-pending after a reported-successful removal while
/// that image section is torn down. One window covers all three.
const FIXTURE_CLEANUP_RETRY_LIMIT: Duration = Duration::from_secs(2);
const FIXTURE_CLEANUP_RETRY_SPACING: Duration = Duration::from_millis(20);

#[cfg(windows)]
fn remove_fixture_binary(binary: &Path, expected: kuru_platform::fs::FileIdentity) -> Result<()> {
    let mut retry_deadline = None;
    loop {
        let (parent, file) = match files::read(binary, Privacy::OwnerOnly) {
            Ok(held) => held,
            // A just-executed Windows image can temporarily deny even the
            // checked read open before we reach the removal syscall. Retry
            // only these native sharing/access denials, within the same
            // deadline as removal and namespace-disappearance observation.
            Err(error)
                if matches!(
                    error
                        .root_cause()
                        .downcast_ref::<io::Error>()
                        .and_then(io::Error::raw_os_error),
                    Some(5 | 32)
                ) =>
            {
                let deadline = *retry_deadline
                    .get_or_insert_with(|| Instant::now() + FIXTURE_CLEANUP_RETRY_LIMIT);
                if Instant::now() >= deadline {
                    return Err(error.context(
                        "fixture cache binary checked reopen exhausted its bounded native removal recovery",
                    ));
                }
                thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(FIXTURE_CLEANUP_RETRY_SPACING),
                );
                continue;
            }
            Err(error) => return Err(error),
        };
        ensure!(
            regular_file_info(&file)
                .context("fixture cache binary held identity inspection")?
                .identity
                == expected,
            "fixture cache binary identity changed before invalidation"
        );
        match parent.remove_file(files::name(binary)?, file) {
            Ok(()) => return wait_for_fixture_binary_absence(binary, &mut retry_deadline),
            // The fixture deletes a cache binary the warm probes just executed,
            // so Windows may refuse either the delete-capable open or the
            // disposition itself while that image section is torn down.
            Err(error)
                if matches!(
                    std::error::Error::source(&error)
                        .and_then(|source| source.downcast_ref::<io::Error>())
                        .and_then(io::Error::raw_os_error),
                    Some(5 | 32)
                ) =>
            {
                let deadline = *retry_deadline
                    .get_or_insert_with(|| Instant::now() + FIXTURE_CLEANUP_RETRY_LIMIT);
                if Instant::now() >= deadline {
                    return Err(anyhow::Error::new(error).context(
                        "fixture cache binary invalidation exhausted its bounded native removal recovery",
                    ));
                }
                thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(FIXTURE_CLEANUP_RETRY_SPACING),
                );
            }
            Err(error) => return Err(anyhow::Error::new(error)),
        }
    }
}

/// A reported-successful removal can still leave the name delete-pending: the
/// just-executed image is torn down asynchronously, and Windows refuses a
/// later create at the same name (native error 5) until the pending delete
/// completes. Wait for the name to become genuinely absent — confirmed only
/// by `NotFound`, never inferred from a successful or a still-denied query —
/// before a caller creates a replacement at it. Bounded by the same deadline
/// `remove_fixture_binary` already established for this removal, so the two
/// waits share one 2-second budget instead of doubling it.
///
/// Not windows-gated: on unix a genuine removal is synchronous, so the first
/// check always observes `NotFound` and this returns immediately. Kept
/// unconditional so the unit tests below can exercise it directly.
fn wait_for_fixture_binary_absence(
    path: &Path,
    retry_deadline: &mut Option<Instant>,
) -> Result<()> {
    let started = Instant::now();
    let mut attempts = 0u32;
    loop {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            // Still present, or still refused as delete-pending: neither is
            // absence. Keep waiting rather than treating a successful stat as
            // proof the name is gone.
            Ok(_) | Err(_) => {}
        }
        attempts += 1;
        let deadline =
            *retry_deadline.get_or_insert_with(|| Instant::now() + FIXTURE_CLEANUP_RETRY_LIMIT);
        if Instant::now() >= deadline {
            bail!(
                "fixture cache binary at {} remained after its reported-successful removal exhausted its bounded native removal recovery, {attempts} reconcile attempts over {:?}",
                path.display(),
                started.elapsed(),
            );
        }
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(FIXTURE_CLEANUP_RETRY_SPACING),
        );
    }
}

#[test]
fn wait_for_fixture_binary_absence_returns_immediately_for_an_absent_path() {
    let root = crate::test_support::tempdir().unwrap();
    let path = root.path().join("never-created");
    let mut deadline = None;
    let started = Instant::now();
    wait_for_fixture_binary_absence(&path, &mut deadline).unwrap();
    assert!(
        started.elapsed() < FIXTURE_CLEANUP_RETRY_SPACING,
        "an already-absent path must not wait at all"
    );
}

#[test]
fn wait_for_fixture_binary_absence_succeeds_once_a_few_polls_observe_absence() {
    let root = crate::test_support::tempdir().unwrap();
    let path = root.path().join("goes-away-after-a-few-polls");
    fs::write(&path, b"present").unwrap();
    let remover = path.clone();
    let releaser = thread::spawn(move || {
        thread::sleep(FIXTURE_CLEANUP_RETRY_SPACING * 2);
        fs::remove_file(&remover).unwrap();
    });
    let mut deadline = None;
    wait_for_fixture_binary_absence(&path, &mut deadline).unwrap();
    releaser.join().unwrap();
    assert!(fs::symlink_metadata(&path).is_err());
}

#[test]
fn wait_for_fixture_binary_absence_exhausts_with_the_established_message() {
    let root = crate::test_support::tempdir().unwrap();
    let path = root.path().join("stays-forever");
    fs::write(&path, b"present").unwrap();
    // Pre-seed an already-elapsed deadline so the bounded window exhausts on
    // its first check instead of spending the full 2-second production
    // budget on a deterministic unit test.
    let mut deadline = Some(Instant::now());
    let error = wait_for_fixture_binary_absence(&path, &mut deadline).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains(&path.display().to_string()), "{rendered}");
    assert!(rendered.contains("1 reconcile attempts"), "{rendered}");
    assert!(
        rendered.contains("exhausted its bounded native removal recovery"),
        "{rendered}"
    );
}

#[tokio::test]
async fn invalid_memory_config_fails_before_provision_creates_cache() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache-not-created");
    let config = MemoryConfig {
        cache_dir: Some("relative-cache".into()),
        ..MemoryConfig::default()
    };

    let error = provision(&config, &cache).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("memory.cache_dir must be an absolute path")
    );
    assert!(!cache.exists());
}

#[test]
fn official_windows_archive_decodes_exact_pinned_payloads_on_every_host() {
    decode_prepared_windows_archive("x86_64-pc-windows-msvc");
}

/// The source-built engine has no download URL. Every Windows on Arm test run
/// first imports the pinned build with `bundle:prepare --archive`, because the
/// memory build refuses that target otherwise, so its real archive is present
/// wherever this compiles; a missing import fails here instead of skipping.
#[cfg(all(windows, target_arch = "aarch64"))]
#[test]
fn source_built_windows_arm64_archive_decodes_exact_pinned_payloads() {
    decode_prepared_windows_archive("aarch64-pc-windows-msvc");
}

/// Decode the real prepared archive for a Windows target and require exactly
/// its pinned payloads. `bundle:test-fixtures` prepares the upstream x64
/// archive on every host. The source-built `aarch64-pc-windows-msvc` archive is
/// decoded on Windows on Arm hosts, which import it; elsewhere its committed
/// catalog entry is checked at the data level below.
fn decode_prepared_windows_archive(target: &str) {
    let asset = crate::catalog::ASSETS
        .iter()
        .find(|asset| asset.target == target)
        .copied()
        .unwrap();
    let bundle_dir = std::env::var_os("KURU_DOLT_BUNDLE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("target")
                .join("kuru-bundles")
        });
    assert!(
        bundle_dir.is_absolute(),
        "KURU_DOLT_BUNDLE_DIR must be absolute"
    );
    let archive_path = bundle_dir.join(format!("{}.archive", asset.archive_sha256));
    let (parent, mut file) = files::read(&archive_path, Privacy::Inherited)
        .expect(
            "prepare the required Windows archive through memory bundle:test-fixtures (x64) or bundle:prepare --archive (the built arm64 engine)",
        );
    assert_eq!(file.metadata().unwrap().len(), asset.compressed_bytes);
    let mut archive = Vec::new();
    (&mut file)
        .take(asset.compressed_bytes + 1)
        .read_to_end(&mut archive)
        .unwrap();
    parent
        .verify(files::name(&archive_path).unwrap(), &file)
        .unwrap();
    let root = crate::test_support::tempdir().unwrap();
    let candidate = root.path().join("real Windows archive");
    extract(&archive, &candidate, asset).unwrap();
    assert_eq!(
        fs::read_dir(&candidate).unwrap().count(),
        2 + asset.notices.len()
    );
    let payloads = [
        ("dolt.exe", asset.executable_bytes, asset.executable_sha256),
        ("LICENSES", asset.license_bytes, asset.license_sha256),
    ]
    .into_iter()
    .chain(
        asset
            .notices
            .iter()
            .map(|notice| (notice.name, notice.bytes, notice.sha256)),
    );
    for (name, size, digest) in payloads {
        let (_parent, file) = files::read(&candidate.join(name), Privacy::OwnerOnly).unwrap();
        kuru_platform::fs::require_private(&file).unwrap();
        assert_eq!(file.metadata().unwrap().len(), size);
        assert_eq!(
            hex_digest(&Sha256::digest(fs::read(candidate.join(name)).unwrap())),
            digest
        );
    }
}

#[test]
fn source_built_windows_arm64_catalog_entry_declares_its_pinned_notices() {
    let asset = crate::catalog::ASSETS
        .iter()
        .find(|asset| asset.target == "aarch64-pc-windows-msvc")
        .copied()
        .expect("the pinned built Windows on Arm engine is catalogued");
    assert_eq!(
        (asset.stem, asset.format, asset.executable_name),
        ("dolt-windows-arm64", "zip", "dolt.exe")
    );
    let x64 = crate::catalog::ASSETS
        .iter()
        .find(|asset| asset.target == "x86_64-pc-windows-msvc")
        .unwrap();
    assert!(
        x64.notices.is_empty(),
        "upstream archives declare no notices"
    );
    // Dolt's own dependency notices are byte-identical to upstream.
    assert_eq!(
        (asset.license_bytes, asset.license_sha256),
        (x64.license_bytes, x64.license_sha256)
    );
    let names: Vec<_> = asset.notices.iter().map(|notice| notice.name).collect();
    assert_eq!(
        names,
        ["LICENSE-ICU", "LICENSE-LLVM", "LICENSE-MINGW-W64-RUNTIME"]
    );
    for notice in asset.notices {
        assert!(notice.bytes > 0 && notice.sha256.len() == 64, "{notice:?}");
    }
    // Extraction requires exactly these members, so the pinned expansion is
    // the executable, LICENSES and every declared notice.
    let notices: u64 = asset.notices.iter().map(|notice| notice.bytes).sum();
    assert_eq!(
        asset.expanded_bytes,
        asset.executable_bytes + asset.license_bytes + notices
    );
    assert!(asset.compressed_bytes <= crate::catalog::MAX_COMPRESSED);
    assert!(asset.expanded_bytes <= crate::catalog::MAX_EXPANDED);
}

fn zip() -> Vec<u8> {
    write(
        &[
            WriteMember {
                name: "fixture/",
                kind: MemberKind::Directory,
                bytes: &[],
                executable: false,
            },
            WriteMember {
                name: "fixture/LICENSES",
                kind: MemberKind::File,
                bytes: NOTICES,
                executable: false,
            },
            WriteMember {
                name: "fixture/bin/",
                kind: MemberKind::Directory,
                bytes: &[],
                executable: false,
            },
            WriteMember {
                name: "fixture/bin/dolt.exe",
                kind: MemberKind::File,
                bytes: EXE,
                executable: true,
            },
        ],
        Limits {
            max_compressed_bytes: 4096,
            max_expanded_bytes: 4096,
            allow_ntfs_timestamps: false,
        },
    )
    .unwrap()
}

fn with_asset<T>(bytes: &[u8], action: impl FnOnce(Asset<'_>) -> T) -> T {
    action(Asset {
        target: "fixture-target",
        stem: "fixture",
        format: "zip",
        executable_name: "dolt.exe",
        compressed_bytes: bytes.len() as u64,
        archive_sha256: &hex_digest(&Sha256::digest(bytes)),
        expanded_bytes: (EXE.len() + NOTICES.len()) as u64,
        executable_bytes: EXE.len() as u64,
        executable_sha256: &hex_digest(&Sha256::digest(EXE)),
        license_bytes: NOTICES.len() as u64,
        license_sha256: &hex_digest(&Sha256::digest(NOTICES)),
        notices: &[],
    })
}

#[test]
fn zip_extraction_owns_exact_payloads_and_never_publishes_unvalidated_bytes() {
    let root = crate::test_support::tempdir().unwrap();
    let archive = zip();
    let candidate = root.path().join("candidate");
    with_asset(&archive, |asset| extract(&archive, &candidate, asset)).unwrap();
    assert_eq!(fs::read(candidate.join("dolt.exe")).unwrap(), EXE);
    assert_eq!(fs::read(candidate.join("LICENSES")).unwrap(), NOTICES);
    assert_eq!(fs::read_dir(&candidate).unwrap().count(), 2);
    let destination = root.path().join("active");
    activate(&candidate, &destination).unwrap();
    assert!(activate(&candidate, &destination).is_err());
    assert_eq!(fs::read(destination.join("dolt.exe")).unwrap(), EXE);
    for name in ["dolt.exe", "LICENSES"] {
        let (_parent, file) = files::read(&destination.join(name), Privacy::OwnerOnly).unwrap();
        kuru_platform::fs::require_private(&file).unwrap();
    }
}

#[tokio::test]
async fn rejected_activation_preserves_verified_stage_and_occupied_destination() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache");
    private_directory(&cache).unwrap();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("runtime");
    let bytes = zip();
    with_asset(&bytes, |asset| extract(&bytes, &candidate, asset)).unwrap();
    let source_identity = files::directory(&candidate).unwrap().identity();
    let destination = cache.join("occupied");
    private_directory(&destination).unwrap();
    files::write(&destination.join("record"), b"unrelated occupant").unwrap();
    let occupied_identity = files::directory(&destination).unwrap().identity();

    // Exclude an in-binary child spawn from copying this flock description
    // while activation releases it and the contender reacquires it.
    let _gate = crate::spawn_gate::locking_async().await;
    let mut recovery_observations = 0_u32;
    let error = activate_staged_observed(stage, lock, &candidate, &destination, |_| {
        recovery_observations += 1;
    })
    .await
    .unwrap_err();
    assert_eq!(
        recovery_observations, 0,
        "an occupied destination never reaches checked recovery"
    );
    assert!(format!("{error:#}").contains("preserved private stage at"));
    assert!(format!("{error:#}").contains(&stage_path.display().to_string()));
    assert!(
        stage_path.is_dir(),
        "the consumed stage owner must retain evidence"
    );
    assert_eq!(
        files::directory(&candidate).unwrap().identity(),
        source_identity
    );
    assert_eq!(fs::read(candidate.join("dolt.exe")).unwrap(), EXE);
    assert_eq!(fs::read(candidate.join("LICENSES")).unwrap(), NOTICES);
    assert_eq!(
        files::directory(&destination).unwrap().identity(),
        occupied_identity
    );
    assert_eq!(
        fs::read(destination.join("record")).unwrap(),
        b"unrelated occupant"
    );
    let contender = open_regular(&cache.join(".install.lock")).unwrap();
    assert_eq!(regular_file_info(&contender).unwrap().identity, identity);
    contender.try_lock().unwrap();
}

#[tokio::test]
async fn activation_source_open_failure_preserves_stage_before_releasing_cache_lock() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache");
    private_directory(&cache).unwrap();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("missing-runtime");
    let destination = cache.join("active");

    // The same release/reacquire pair must exclude transient inherited flock
    // copies; the initial lock acquisition above used its own short guard.
    let _gate = crate::spawn_gate::locking_async().await;
    let error = activate_staged(stage, lock, &candidate, &destination)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("open verified Dolt activation source"));
    assert!(format!("{error:#}").contains("preserved private stage at"));
    assert!(stage_path.is_dir());
    assert!(!destination.exists());
    let reacquired = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
    assert_eq!(
        regular_file_info(&reacquired).unwrap().identity,
        lock_identity
    );
    drop(reacquired);
}

#[tokio::test]
async fn successful_activation_removes_its_disposable_stage() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache");
    private_directory(&cache).unwrap();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_container = stage.path().parent().unwrap().to_owned();
    let candidate = stage.path().join("runtime");
    let bytes = zip();
    with_asset(&bytes, |asset| extract(&bytes, &candidate, asset)).unwrap();
    let destination = cache.join("active");

    assert!(
        activate_staged(stage, lock, &candidate, &destination)
            .await
            .unwrap()
            .is_none(),
        "a removed stage reports no retained leftover"
    );

    assert_eq!(fs::read(destination.join("dolt.exe")).unwrap(), EXE);
    assert_eq!(fs::read(destination.join("LICENSES")).unwrap(), NOTICES);
    assert!(!stage_container.exists());
    assert_eq!(
        fs::read_dir(&cache)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".install-"))
            .count(),
        0
    );
}

#[cfg(windows)]
#[tokio::test]
async fn published_engine_retains_its_failed_stage_cleanup_and_releases_cache_lease() {
    use std::os::windows::fs::OpenOptionsExt;

    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache");
    private_directory(&cache).unwrap();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_container = stage.path().parent().unwrap().to_owned();
    let stage_identity = files::directory(stage.path()).unwrap().identity();
    let candidate = stage.path().join("runtime");
    let bytes = zip();
    with_asset(&bytes, |asset| extract(&bytes, &candidate, asset)).unwrap();
    let blocker_path = stage.path().join("cleanup-blocker");
    files::write(&blocker_path, b"fixture-only blocker").unwrap();
    // This sibling does not block the checked candidate move; denying delete
    // sharing makes only the disposable stage close fail on Windows.
    let blocker = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x3)
        .open(&blocker_path)
        .unwrap();
    let destination = cache.join("active");

    let failure = activate_staged(stage, lock, &candidate, &destination)
        .await
        .unwrap()
        .expect("a published engine reports its retained stage instead of failing");
    assert_eq!(failure.stage, stage_container);
    let detail = failure.first_cause.clone();
    assert!(detail.contains("Dolt engine publication succeeded, but private stage cleanup failed"));
    assert!(detail.contains(&stage_container.display().to_string()));
    assert!(
        detail.contains("(os error 32)"),
        "the persistent no-DELETE holder must retain the native cleanup cause: {detail}"
    );
    assert_eq!(fs::read(destination.join("dolt.exe")).unwrap(), EXE);
    assert_eq!(fs::read(destination.join("LICENSES")).unwrap(), NOTICES);
    assert!(blocker_path.exists());
    assert_eq!(
        files::directory(stage_container.join("private").as_path())
            .unwrap()
            .identity(),
        stage_identity,
        "the exhausted cleanup must preserve the original private stage"
    );
    let reacquired = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        regular_file_info(&reacquired).unwrap().identity,
        lock_identity
    );
    drop(reacquired);

    drop(blocker);
    fs::remove_dir_all(&stage_container).unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn held_cold_probe_copy_does_not_block_candidate_activation() {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;

    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    private_directory(&cache).unwrap();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let stage_container = stage_path.parent().unwrap().to_owned();
    let candidate = stage_path.join("runtime");
    extract(EMBEDDED_ARCHIVE, &candidate, BUNDLED_ASSET).unwrap();
    let source_identity = files::directory(&candidate).unwrap().identity();
    let candidate_binary = candidate.join(BUNDLED_ASSET.executable_name);
    let (candidate_parent, candidate_file) =
        files::read(&candidate_binary, Privacy::OwnerOnly).unwrap();
    let candidate_binary_identity = regular_file_info(&candidate_file).unwrap().identity;
    drop(candidate_file);
    drop(candidate_parent);

    let probe_home = stage_path.join("probe");
    let mut observed_probe_identity = None;
    let probe = prepare_cold_probe_observed(&candidate, &probe_home, BUNDLED_ASSET, |file| {
        let identity = regular_file_info(file)?.identity;
        assert_ne!(
            identity, candidate_binary_identity,
            "cold probe image must have a distinct filesystem identity"
        );
        observed_probe_identity = Some(identity);
        Ok(())
    })
    .unwrap();
    let probe_binary = probe.binary.clone();
    assert!(
        !probe_binary.starts_with(&candidate),
        "cold probe image must be outside the candidate directory being activated"
    );
    let (probe_parent, probe_file) = files::read(&probe_binary, Privacy::OwnerOnly).unwrap();
    assert_eq!(
        regular_file_info(&probe_file).unwrap().identity,
        observed_probe_identity.expect("verified copy observer ran"),
        "the checked probe copy changed before it was opened for the native hold"
    );
    drop(probe_file);
    drop(probe_parent);

    // Permit the actual version probe to read and execute its isolated copy,
    // but model a loaded Windows image by denying DELETE sharing. This known
    // condition must affect only the probe copy, never the activation source.
    let blocker = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&probe_binary)
        .unwrap();
    let renamed_probe = probe_binary.with_file_name("probe-image-renamed.exe");
    let error = fs::rename(&probe_binary, &renamed_probe)
        .expect_err("retained probe image handle must deny renaming its own copy");
    assert_eq!(error.raw_os_error(), Some(32));
    assert!(probe_binary.is_file());
    assert!(!renamed_probe.exists());

    let (probe, lease) = probe
        .probe(StageLease::new(stage, lock, BUNDLED_ASSET))
        .await
        .unwrap();
    let destination = cache.join("active");
    let failure = activate_staged_after_probe(lease, probe, &candidate, &destination)
        .await
        .unwrap()
        .expect("a published engine reports its retained stage instead of failing");
    assert_eq!(failure.stage, stage_container);
    let detail = failure.first_cause.clone();
    assert!(detail.contains("Dolt engine publication succeeded, but private stage cleanup failed"));
    assert!(detail.contains(&stage_path.display().to_string()));
    assert!(
        detail.contains("(os error 32)"),
        "the retained no-DELETE probe must be the Windows sharing violation: {detail}"
    );
    assert_eq!(
        files::directory(&destination).unwrap().identity(),
        source_identity
    );
    assert!(!candidate.exists());
    verify_payload(
        &destination.join(BUNDLED_ASSET.executable_name),
        BUNDLED_ASSET.executable_bytes,
        BUNDLED_ASSET.executable_sha256,
        true,
    )
    .unwrap();
    verify_payload(
        &destination.join("LICENSES"),
        BUNDLED_ASSET.license_bytes,
        BUNDLED_ASSET.license_sha256,
        false,
    )
    .unwrap();
    assert!(
        blocker.metadata().is_ok(),
        "the isolated no-DELETE probe handle must remain held through activation"
    );
    let reacquired = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    drop(reacquired);
    drop(blocker);
    fs::remove_dir_all(&stage_container).unwrap();
    assert!(!stage_container.exists());
}

#[cfg(windows)]
#[tokio::test]
async fn held_descendant_releases_after_checked_no_move_and_activation_recovers() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    private_directory(&cache).unwrap();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("runtime");
    extract(EMBEDDED_ARCHIVE, &candidate, BUNDLED_ASSET).unwrap();
    gated_verify_version(
        &candidate.join(BUNDLED_ASSET.executable_name),
        &stage_path.join("probe"),
    )
    .await
    .unwrap();
    let source_identity = files::directory(&candidate).unwrap().identity();
    // Even a delete-sharing data handle prevents moving its containing Windows
    // directory. The real probe already exited before this known blocker opens.
    let (parent, blocker) = files::read(&candidate.join("LICENSES"), Privacy::OwnerOnly).unwrap();
    // The parent was needed only to establish the checked read. Retaining it
    // would keep an ancestor of the disposable stage open after the leaf
    // blocker is released.
    drop(parent);
    let destination = cache.join("active");
    let lock_path = cache.join(".install.lock");
    let mut blocker = Some(blocker);
    let mut denied = 0_u32;
    // The native move and reconciliation are synchronous. Keep the real probe
    // above, then isolate only this positive retry from runner wall-clock load.
    tokio::time::pause();
    let result =
        activate_staged_observed(stage, lock, &candidate, &destination, |proven_no_move| {
            if !proven_no_move {
                return;
            }
            denied += 1;
            let contender = open_regular(&lock_path).unwrap();
            assert_eq!(
                regular_file_info(&contender).unwrap().identity,
                lock_identity
            );
            assert!(matches!(
                contender.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            drop(blocker.take());
        })
        .await;
    tokio::time::resume();
    assert!(result.unwrap().is_none());
    assert!(
        denied >= 1,
        "release the blocker on an observed checked no-move refusal"
    );
    assert_eq!(
        files::directory(&destination).unwrap().identity(),
        source_identity
    );
    assert!(!candidate.exists());
    verify_payload(
        &destination.join(BUNDLED_ASSET.executable_name),
        BUNDLED_ASSET.executable_bytes,
        BUNDLED_ASSET.executable_sha256,
        true,
    )
    .unwrap();
    verify_payload(
        &destination.join("LICENSES"),
        BUNDLED_ASSET.license_bytes,
        BUNDLED_ASSET.license_sha256,
        false,
    )
    .unwrap();
    let contender = open_regular(&lock_path).unwrap();
    assert_eq!(
        regular_file_info(&contender).unwrap().identity,
        lock_identity
    );
    {
        // Held across the actual flock acquisition this assertion proves
        // succeeds; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        contender.try_lock().unwrap();
    }
}

#[cfg(windows)]
#[tokio::test]
async fn persistent_held_descendant_exhausts_checked_recovery_and_preserves_stage() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    private_directory(&cache).unwrap();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("runtime");
    extract(EMBEDDED_ARCHIVE, &candidate, BUNDLED_ASSET).unwrap();
    gated_verify_version(
        &candidate.join(BUNDLED_ASSET.executable_name),
        &stage_path.join("probe"),
    )
    .await
    .unwrap();
    let source_identity = files::directory(&candidate).unwrap().identity();
    let (_parent, _blocker) = files::read(&candidate.join("LICENSES"), Privacy::OwnerOnly).unwrap();
    let destination = cache.join("active");
    let lock_path = cache.join(".install.lock");
    let mut checked_denials = 0_u32;
    let started = std::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(4),
        activate_staged_observed(stage, lock, &candidate, &destination, |proven_no_move| {
            assert!(proven_no_move, "only checked no-move may enter recovery");
            checked_denials += 1;
            let contender = open_regular(&lock_path).unwrap();
            assert_eq!(
                regular_file_info(&contender).unwrap().identity,
                lock_identity
            );
            assert!(matches!(
                contender.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
        }),
    )
    .await
    .expect("bounded recovery must not exceed its fixture deadline")
    .unwrap_err();
    assert!(
        started.elapsed() >= ACTIVATION_RETRY_LIMIT,
        "the persistent blocker must exercise the bounded recovery window"
    );
    assert!(
        checked_denials >= 1,
        "the held descendant must deny a checked move"
    );
    let publication = error
        .downcast_ref::<kuru_platform::fs::PublicationError>()
        .unwrap();
    assert_eq!(
        publication.phase,
        kuru_platform::fs::PublicationPhase::Rejected
    );
    assert_eq!(publication.error().raw_os_error(), Some(5));
    let diagnostic = format!("{error:#}");
    assert!(diagnostic.contains("runtime activation recovery stopped"));
    assert!(diagnostic.contains("preserved private stage at"));
    assert!(stage_path.is_dir());
    assert_eq!(
        files::directory(&candidate).unwrap().identity(),
        source_identity
    );
    assert!(!destination.exists());
    let contender = open_regular(&lock_path).unwrap();
    assert_eq!(
        regular_file_info(&contender).unwrap().identity,
        lock_identity
    );
    {
        // Held across the actual flock acquisition this assertion proves
        // succeeds; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        contender.try_lock().unwrap();
    }
}

#[cfg(windows)]
#[tokio::test]
async fn cancelling_checked_activation_recovery_drops_stage_before_cache_lock() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    private_directory(&cache).unwrap();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("runtime");
    extract(EMBEDDED_ARCHIVE, &candidate, BUNDLED_ASSET).unwrap();
    gated_verify_version(
        &candidate.join(BUNDLED_ASSET.executable_name),
        &stage_path.join("probe"),
    )
    .await
    .unwrap();
    // The `runtime` directory handle `files::read` returns is released with
    // its blocker: under the checked (legacy-disposition) stage removal a
    // still-open handle keeps `runtime` delete-pending, which the unchecked
    // POSIX removal this test was written against had bypassed.
    let blocker = files::read(&candidate.join("LICENSES"), Privacy::OwnerOnly).unwrap();
    let destination = cache.join("active");
    let lock_path = cache.join(".install.lock");
    let abort_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<tokio::task::AbortHandle>));
    let observer_abort_slot = abort_slot.clone();
    let task_destination = destination.clone();
    // The native move and reconciliation are synchronous and have no product
    // time bound, and recovery's only await point is its retry-spacing wait,
    // taken only while the tokio clock is inside the recovery window. Keep the
    // real probe above, then isolate this cancellation from runner wall-clock
    // load exactly as `held_descendant_releases_after_checked_no_move_and_activation_recovers`
    // isolates its retry: a slow first move cannot close the window before the
    // cancellation reaches that wait. Stage teardown on drop keeps its own
    // wall-clock bound (`std::time::Instant` in `files.rs`), which pausing
    // tokio time does not touch.
    tokio::time::pause();
    let task = tokio::spawn(async move {
        let mut blocker = Some(blocker);
        activate_staged_observed(
            stage,
            lock,
            &candidate,
            &task_destination,
            move |proven_no_move| {
                if proven_no_move {
                    drop(blocker.take());
                    observer_abort_slot
                        .lock()
                        .unwrap()
                        .as_ref()
                        .expect("abort handle is installed before the task runs")
                        .abort();
                }
            },
        )
        .await
    });
    *abort_slot.lock().unwrap() = Some(task.abort_handle());
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::resume();
    assert!(!destination.exists());
    assert!(
        !stage_path.exists(),
        "cancellation drops the private stage before releasing the cache lock"
    );
    let contender = open_regular(&lock_path).unwrap();
    assert_eq!(
        regular_file_info(&contender).unwrap().identity,
        lock_identity
    );
    {
        // Held across the actual flock acquisition this assertion proves
        // succeeds; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        contender.try_lock().unwrap();
    }
}

/// A zip-fixture stage under `cache` with its `runtime` candidate extracted,
/// the checked installation lock held, and that lock's identity.
#[cfg(windows)]
async fn fixture_activation_stage(
    cache: &Path,
) -> (
    PrivateTemp,
    CacheLock,
    kuru_platform::fs::FileIdentity,
    PathBuf,
) {
    private_directory(cache).unwrap();
    let lock = gated_cache_lock(cache, Duration::from_secs(1))
        .await
        .unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(cache)).unwrap();
    let candidate = stage.path().join("runtime");
    let bytes = zip();
    with_asset(&bytes, |asset| extract(&bytes, &candidate, asset)).unwrap();
    (stage, lock, lock_identity, candidate)
}

#[cfg(windows)]
async fn assert_cache_lock_released(
    lock_path: &Path,
    lock_identity: kuru_platform::fs::FileIdentity,
) {
    let contender = open_regular(lock_path).unwrap();
    assert_eq!(
        regular_file_info(&contender).unwrap().identity,
        lock_identity
    );
    // Held across the actual flock acquisition this assertion proves
    // succeeds; see `crate::spawn_gate`.
    let _gate = crate::spawn_gate::locking_async().await;
    contender.try_lock().unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn cancelled_activation_recovery_receipts_a_held_stage_before_releasing_the_lock() {
    use std::os::windows::fs::OpenOptionsExt;

    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    let (stage, lock, lock_identity, candidate) = fixture_activation_stage(&cache).await;
    let stage_path = stage.path().to_owned();
    let stage_container = stage_path.parent().unwrap().to_owned();
    let stage_identity = files::directory(&stage_path).unwrap().identity();
    let holder_path = stage_path.join("teardown-holder");
    files::write(&holder_path, b"fixture-only teardown holder").unwrap();
    // A sibling of the candidate does not block its checked move. Denying
    // delete sharing refuses the stage's checked removal for the whole bounded
    // window, as a scanner's handle would for part of it.
    let holder = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x3)
        .open(&holder_path)
        .unwrap();
    let held = files::read(&candidate.join("LICENSES"), Privacy::OwnerOnly).unwrap();
    let destination = cache.join("active");
    let lock_path = cache.join(".install.lock");
    let (observer, observed) = observe_retained_stage_reports(&cache, &lock_path);
    let abort_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<tokio::task::AbortHandle>));
    let observer_abort_slot = abort_slot.clone();
    let task_destination = destination.clone();
    // As in `cancelling_checked_activation_recovery_drops_stage_before_cache_lock`,
    // the native move and reconciliation are synchronous and have no product
    // time bound, and recovery's only await point is its retry-spacing wait,
    // taken only while the tokio clock is inside the recovery window. Keep the
    // real lock acquisition above, then isolate this cancellation from runner
    // wall-clock load: a slow first move cannot close the window before the
    // cancellation reaches that wait. The held stage's bounded teardown on drop
    // keeps its own wall-clock bound (`std::time::Instant` in `files.rs`),
    // which pausing tokio time does not touch.
    tokio::time::pause();
    let task = tokio::spawn(async move {
        let mut held = Some(held);
        activate_staged_observed(
            stage,
            lock,
            &candidate,
            &task_destination,
            move |proven_no_move| {
                if proven_no_move {
                    drop(held.take());
                    observer_abort_slot
                        .lock()
                        .unwrap()
                        .as_ref()
                        .expect("abort handle is installed before the task runs")
                        .abort();
                }
            },
        )
        .await
    });
    *abort_slot.lock().unwrap() = Some(task.abort_handle());
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::resume();
    drop(observer);
    assert!(!destination.exists());
    let observed = std::mem::take(&mut *observed.lock().unwrap());
    assert_eq!(observed.len(), 1, "one retention report: {observed:?}");
    assert_eq!(observed[0].stage, stage_container);
    assert_eq!(
        observed[0].message, "retained private install stage after an unpublished installation",
        "{observed:?}"
    );
    assert_eq!(observed[0].published, Some(false), "{observed:?}");
    assert!(
        observed[0].receipted && observed[0].lock_held,
        "the held stage is receipted and reported before the cache lock is released: {observed:?}"
    );
    let receipts = leftover_stage_receipts(&cache.join(LEFTOVER_STAGE_RECEIPTS));
    assert_eq!(receipts.len(), 1, "exactly one retained stage is receipted");
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipts[0]).unwrap()).unwrap();
    assert_eq!(receipt["published"], false);
    assert_eq!(
        receipt["stage"],
        stage_container
            .file_name()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    let cause = receipt["first_cause"].as_str().unwrap();
    assert!(
        cause.contains("exhausted its bounded recovery"),
        "the persistent holder exhausts the bounded stage removal: {cause}"
    );
    assert!(
        receipt["os_error"].is_i64(),
        "the refusal's native cause is retained: {receipt}"
    );
    assert!(holder_path.exists());
    assert_eq!(
        files::directory(&stage_path).unwrap().identity(),
        stage_identity,
        "a refused teardown preserves the original private stage"
    );
    assert_cache_lock_released(&lock_path, lock_identity).await;
    drop(holder);
    fs::remove_dir_all(&stage_container).unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn first_checked_no_move_after_the_window_reports_stopped_recovery() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    let (stage, lock, lock_identity, candidate) = fixture_activation_stage(&cache).await;
    let stage_path = stage.path().to_owned();
    let source_identity = files::directory(&candidate).unwrap().identity();
    let (_parent, _blocker) = files::read(&candidate.join("LICENSES"), Privacy::OwnerOnly).unwrap();
    let destination = cache.join("active");
    let lock_path = cache.join(".install.lock");
    let mut checked_denials = 0_u32;
    let error = activate_staged_observed(stage, lock, &candidate, &destination, |proven_no_move| {
        assert!(proven_no_move, "only checked no-move may enter recovery");
        checked_denials += 1;
        if checked_denials == 1 {
            // Fixture delay, not a product one: the first native attempt's
            // result arrives only after the whole recovery window, as a
            // slow first move under a loaded runner did in CI.
            thread::sleep(ACTIVATION_RETRY_LIMIT);
        }
    })
    .await
    .unwrap_err();
    assert_eq!(
        checked_denials, 1,
        "no native move starts after the recovery window"
    );
    let publication = error
        .downcast_ref::<kuru_platform::fs::PublicationError>()
        .unwrap();
    assert_eq!(
        publication.phase,
        kuru_platform::fs::PublicationPhase::Rejected
    );
    assert_eq!(publication.error().raw_os_error(), Some(5));
    let diagnostic = format!("{error:#}");
    assert!(
        diagnostic.contains("runtime activation recovery stopped"),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains("preserved private stage at"),
        "{diagnostic}"
    );
    assert!(stage_path.is_dir());
    assert_eq!(
        files::directory(&candidate).unwrap().identity(),
        source_identity
    );
    assert!(!destination.exists());
    assert_cache_lock_released(&lock_path, lock_identity).await;
}

/// A caller cancelled as a late first checked no-move is observed (job
/// 108878373677): the recovery window has already closed, so the loop reaches
/// no await point before its terminal decision. The cancellation is honoured
/// only at recovery's existing await (the retry-spacing wait, covered by
/// `cancelling_checked_activation_recovery_drops_stage_before_cache_lock`);
/// here the operation completes with stopped recovery, names the preserved
/// stage, and releases the cache lock only after that stage is kept.
#[cfg(windows)]
#[tokio::test]
async fn cancellation_at_a_late_first_checked_no_move_completes_with_stopped_recovery() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    let (stage, lock, lock_identity, candidate) = fixture_activation_stage(&cache).await;
    let stage_path = stage.path().to_owned();
    let source_identity = files::directory(&candidate).unwrap().identity();
    let held = files::read(&candidate.join("LICENSES"), Privacy::OwnerOnly).unwrap();
    let destination = cache.join("active");
    let lock_path = cache.join(".install.lock");
    let abort_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<tokio::task::AbortHandle>));
    let observer_abort_slot = abort_slot.clone();
    let task_destination = destination.clone();
    let task_candidate = candidate.clone();
    let observer_lock_path = lock_path.clone();
    let checked_denials = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let observer_denials = checked_denials.clone();
    let task = tokio::spawn(async move {
        let mut held = Some(held);
        activate_staged_observed(
            stage,
            lock,
            &task_candidate,
            &task_destination,
            move |proven_no_move| {
                assert!(proven_no_move, "only checked no-move may enter recovery");
                observer_denials.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // Fixture delay, not a product one: the first result arrives
                // after the recovery window (job 108878373677).
                thread::sleep(ACTIVATION_RETRY_LIMIT);
                let contender = open_regular(&observer_lock_path).unwrap();
                assert_eq!(
                    regular_file_info(&contender).unwrap().identity,
                    lock_identity
                );
                assert!(matches!(
                    contender.try_lock(),
                    Err(TryLockError::WouldBlock)
                ));
                drop(held.take());
                observer_abort_slot
                    .lock()
                    .unwrap()
                    .as_ref()
                    .expect("abort handle is installed before the task runs")
                    .abort();
            },
        )
        .await
    });
    *abort_slot.lock().unwrap() = Some(task.abort_handle());
    let error = task
        .await
        .expect("no await point follows a late terminal no-move, so the task completes")
        .unwrap_err();
    assert_eq!(
        checked_denials.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "no native move starts after the recovery window"
    );
    let publication = error
        .downcast_ref::<kuru_platform::fs::PublicationError>()
        .unwrap();
    assert_eq!(
        publication.phase,
        kuru_platform::fs::PublicationPhase::Rejected
    );
    assert_eq!(publication.error().raw_os_error(), Some(5));
    let diagnostic = format!("{error:#}");
    assert!(
        diagnostic.contains("runtime activation recovery stopped"),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains("preserved private stage at"),
        "{diagnostic}"
    );
    assert!(stage_path.is_dir());
    assert_eq!(
        files::directory(&candidate).unwrap().identity(),
        source_identity
    );
    assert!(!destination.exists());
    assert_cache_lock_released(&lock_path, lock_identity).await;
}

#[test]
fn physical_zip_mode_size_inventory_and_payload_pins_are_enforced_before_activation() {
    let root = crate::test_support::tempdir().unwrap();
    let original = zip();
    let central = original
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    for (index, value) in [
        (central + 38, 0x08_u8),
        (central + 41, 0xa0),
        (central + 28, 1),
        (central + 24, 1),
    ] {
        let mut changed = original.clone();
        changed[index] = value;
        // Re-pin the changed archive so this proves structural/domain checks,
        // not merely the enclosing checksum guard.
        with_asset(&changed, |asset| {
            assert!(extract(&changed, &root.path().join(format!("bad-{index}")), asset).is_err());
        });
    }
    with_asset(&original, |mut asset| {
        asset.executable_sha256 = asset.license_sha256;
        assert!(extract(&original, &root.path().join("wrong-exe-digest"), asset).is_err());
    });
    with_asset(&original, |mut asset| {
        asset.license_sha256 = asset.executable_sha256;
        assert!(extract(&original, &root.path().join("wrong-notice-digest"), asset).is_err());
    });
    with_asset(&original, |mut asset| {
        asset.executable_name = "unexpected.exe";
        assert!(extract(&original, &root.path().join("wrong-member"), asset).is_err());
    });
    with_asset(&original, |mut asset| {
        asset.expanded_bytes -= 1;
        assert!(extract(&original, &root.path().join("expanded-limit"), asset).is_err());
    });
    assert!(!root.path().join("active").exists());
}

#[tokio::test]
async fn cache_lease_uses_actual_identity_and_retains_contention_until_owner_drops() {
    let root = crate::test_support::tempdir().unwrap();
    let first = gated_cache_lock(root.path(), Duration::from_secs(1))
        .await
        .unwrap();
    let identity = regular_file_info(&first).unwrap().identity;
    assert!(gated_cache_lock(root.path(), Duration::ZERO).await.is_err());
    drop(first);
    let second = gated_cache_lock(root.path(), Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(regular_file_info(&second).unwrap().identity, identity);
}

#[tokio::test]
async fn actual_warm_cache_verifies_concurrently_while_installation_lock_is_held() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("actual concurrent warm cache café 東京");
    let config = MemoryConfig {
        offline: true,
        cache_dir: Some(cache.clone()),
        ..MemoryConfig::default()
    };
    let cold_started = std::time::Instant::now();
    let binary = provision(&config, &cache).await.unwrap();
    let cold_elapsed = cold_started.elapsed();
    let lock = gated_cache_lock(&cache, Duration::from_secs(1))
        .await
        .unwrap();
    let warm_started = std::time::Instant::now();
    let concurrent = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(provision(&config, &cache), provision(&config, &cache))
    })
    .await
    .expect("actual warm opens must not wait for installation authority");
    let warm_elapsed = warm_started.elapsed();
    assert_eq!(concurrent.0.unwrap(), binary);
    assert_eq!(concurrent.1.unwrap(), binary);
    eprintln!(
        "observed actual managed provisioning: cold={cold_elapsed:?}; two concurrent warm opens={warm_elapsed:?}; full payload digests and version probes retained"
    );
    drop(lock);

    #[cfg(unix)]
    {
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let parent = files::parent(&binary, Privacy::OwnerOnly, NameRetention::Movable).unwrap();
        let mut corrupt = parent.read_write(files::name(&binary).unwrap()).unwrap();
        corrupt.seek(SeekFrom::Start(0)).unwrap();
        corrupt.write_all(b"!").unwrap();
        corrupt.sync_all().unwrap();
        parent
            .verify(files::name(&binary).unwrap(), &corrupt)
            .unwrap();
    }
    #[cfg(windows)]
    {
        // Installed Windows payloads are deliberately sealed owner-read/execute.
        // Replace this isolated fixture with private bytes of the expected size
        // rather than weakening the production ACL to make it writable.
        let (_parent, file) = files::read(&binary, Privacy::OwnerOnly).unwrap();
        let identity = regular_file_info(&file).unwrap().identity;
        drop(file);
        drop(_parent);
        remove_fixture_binary(&binary, identity).unwrap();
        let corrupt = new_private_file(&binary).unwrap();
        corrupt.set_len(BUNDLED_ASSET.executable_bytes).unwrap();
        corrupt.sync_all().unwrap();
    }
    let error = provision(&config, &cache).await.unwrap_err();
    assert!(format!("{error:#}").contains("payload checksum mismatch"));
}

#[cfg(windows)]
#[tokio::test]
async fn concurrent_cold_windows_provision_publishes_one_verified_native_identity() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("concurrent empty cache café 東京");
    let config = MemoryConfig {
        offline: true,
        cache_dir: Some(cache.clone()),
        ..Default::default()
    };
    let start = tokio::sync::Barrier::new(2);
    let open = || async {
        start.wait().await;
        let binary = provision(&config, &cache).await.unwrap();
        let (_parent, file) = files::read(&binary, Privacy::OwnerOnly).unwrap();
        (binary, regular_file_info(&file).unwrap().identity)
    };
    // Provisioning supplies bounded lock/probe waits. Retain this root until
    // both calls finish, including their independently owned extraction workers.
    let (first, second) = tokio::join!(open(), open());
    assert_eq!(first, second);
    for (name, size, digest) in [
        (
            "dolt.exe",
            BUNDLED_ASSET.executable_bytes,
            BUNDLED_ASSET.executable_sha256,
        ),
        (
            "LICENSES",
            BUNDLED_ASSET.license_bytes,
            BUNDLED_ASSET.license_sha256,
        ),
    ] {
        let path = first.0.with_file_name(name);
        let (_parent, file) = files::read(&path, Privacy::OwnerOnly).unwrap();
        kuru_platform::fs::require_private(&file).unwrap();
        assert_eq!(file.metadata().unwrap().len(), size);
        assert_eq!(hex_digest(&Sha256::digest(fs::read(path).unwrap())), digest);
    }
    let warm = provision(&config, &cache).await.unwrap();
    let (_parent, file) = files::read(&warm, Privacy::OwnerOnly).unwrap();
    assert_eq!(warm, first.0);
    assert_eq!(regular_file_info(&file).unwrap().identity, first.1);
    let entries: Vec<_> = fs::read_dir(cache.join(DOLT_VERSION))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, [std::ffi::OsString::from(BUNDLED_ASSET.target)]);
}

#[cfg(windows)]
#[tokio::test]
async fn real_embedded_windows_engine_installs_offline_and_corrupt_cache_fails_before_execution() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("empty cache 東京");
    let config = MemoryConfig {
        offline: true,
        cache_dir: Some(cache.clone()),
        ..Default::default()
    };
    let binary = provision(&config, &cache).await.unwrap();
    assert_eq!(binary.file_name().unwrap(), "dolt.exe");
    let (_parent, file) = files::read(&binary, Privacy::OwnerOnly).unwrap();
    let identity = regular_file_info(&file).unwrap().identity;
    drop(file);
    assert_eq!(provision(&config, &cache).await.unwrap(), binary);
    let (_parent, file) = files::read(&binary, Privacy::OwnerOnly).unwrap();
    assert_eq!(regular_file_info(&file).unwrap().identity, identity);
    drop(file);
    assert_eq!(
        hex_digest(&Sha256::digest(
            fs::read(binary.with_file_name("LICENSES")).unwrap()
        )),
        BUNDLED_ASSET.license_sha256
    );
    remove_fixture_binary(&binary, identity).unwrap();
    let file = new_private_file(&binary).unwrap();
    file.set_len(BUNDLED_ASSET.executable_bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let error = provision(&config, &cache).await.unwrap_err();
    assert!(format!("{error:#}").contains("checksum"), "{error:#}");
    assert_eq!(
        fs::metadata(&binary).unwrap().len(),
        BUNDLED_ASSET.executable_bytes
    );
}

const ICU_NOTICE: &[u8] = b"fixture ICU license notice";
const LLVM_NOTICE: &[u8] = b"fixture LLVM runtime license notice";

/// A source-built archive in the manifest layout: the four upstream members,
/// then each declared third-party notice beside `LICENSES`.
fn built_zip(notices: &[(&str, &[u8])]) -> Vec<u8> {
    let names: Vec<_> = notices
        .iter()
        .map(|(name, _)| format!("fixture/{name}"))
        .collect();
    let mut members = vec![
        WriteMember {
            name: "fixture/",
            kind: MemberKind::Directory,
            bytes: &[],
            executable: false,
        },
        WriteMember {
            name: "fixture/bin/",
            kind: MemberKind::Directory,
            bytes: &[],
            executable: false,
        },
        WriteMember {
            name: "fixture/bin/dolt.exe",
            kind: MemberKind::File,
            bytes: EXE,
            executable: true,
        },
        WriteMember {
            name: "fixture/LICENSES",
            kind: MemberKind::File,
            bytes: NOTICES,
            executable: false,
        },
    ];
    members.extend(
        names
            .iter()
            .zip(notices)
            .map(|(name, (_, bytes))| WriteMember {
                name,
                kind: MemberKind::File,
                bytes,
                executable: false,
            }),
    );
    write(
        &members,
        Limits {
            max_compressed_bytes: 4096,
            max_expanded_bytes: 4096,
            allow_ntfs_timestamps: false,
        },
    )
    .unwrap()
}

fn with_built_asset<T>(
    bytes: &[u8],
    notices: &[crate::catalog::Notice<'_>],
    action: impl FnOnce(Asset<'_>) -> T,
) -> T {
    let declared: u64 = notices.iter().map(|notice| notice.bytes).sum();
    action(Asset {
        target: "fixture-built-target",
        stem: "fixture",
        format: "zip",
        executable_name: "dolt.exe",
        compressed_bytes: bytes.len() as u64,
        archive_sha256: &hex_digest(&Sha256::digest(bytes)),
        expanded_bytes: (EXE.len() + NOTICES.len()) as u64 + declared,
        executable_bytes: EXE.len() as u64,
        executable_sha256: &hex_digest(&Sha256::digest(EXE)),
        license_bytes: NOTICES.len() as u64,
        license_sha256: &hex_digest(&Sha256::digest(NOTICES)),
        notices,
    })
}

#[test]
fn built_zip_extraction_accepts_exactly_the_declared_notices() {
    let icu_digest = hex_digest(&Sha256::digest(ICU_NOTICE));
    let llvm_digest = hex_digest(&Sha256::digest(LLVM_NOTICE));
    let declared = [
        crate::catalog::Notice {
            name: "LICENSE-ICU",
            bytes: ICU_NOTICE.len() as u64,
            sha256: &icu_digest,
        },
        crate::catalog::Notice {
            name: "LICENSE-LLVM",
            bytes: LLVM_NOTICE.len() as u64,
            sha256: &llvm_digest,
        },
    ];
    let root = crate::test_support::tempdir().unwrap();
    let archive = built_zip(&[("LICENSE-ICU", ICU_NOTICE), ("LICENSE-LLVM", LLVM_NOTICE)]);
    let candidate = root.path().join("candidate");
    with_built_asset(&archive, &declared, |asset| {
        extract(&archive, &candidate, asset)
    })
    .unwrap();
    assert_eq!(fs::read_dir(&candidate).unwrap().count(), 4);
    assert_eq!(fs::read(candidate.join("dolt.exe")).unwrap(), EXE);
    assert_eq!(fs::read(candidate.join("LICENSES")).unwrap(), NOTICES);
    assert_eq!(fs::read(candidate.join("LICENSE-ICU")).unwrap(), ICU_NOTICE);
    assert_eq!(
        fs::read(candidate.join("LICENSE-LLVM")).unwrap(),
        LLVM_NOTICE
    );
    for name in ["LICENSE-ICU", "LICENSE-LLVM"] {
        let (_parent, file) = files::read(&candidate.join(name), Privacy::OwnerOnly).unwrap();
        kuru_platform::fs::require_private(&file).unwrap();
    }
    let owned: Vec<_> = declared
        .iter()
        .map(|notice| {
            (
                notice.name.to_owned(),
                notice.bytes,
                notice.sha256.to_owned(),
            )
        })
        .collect();
    let checked = with_built_asset(&archive, &declared, |asset| {
        CheckedCache::open_and_verify(
            &candidate,
            asset.executable_name,
            asset.executable_bytes,
            asset.executable_sha256,
            asset.license_bytes,
            asset.license_sha256,
            &owned,
        )
    })
    .unwrap();
    checked.revalidate().unwrap();
    assert_eq!(checked.notices.len(), 2);
    drop(checked);
    // A cached notice that changed after extraction fails verification.
    fs::remove_file(candidate.join("LICENSE-LLVM")).unwrap();
    files::write(
        &candidate.join("LICENSE-LLVM"),
        b"tampered notice bytes!!!!!!!!!!!!!",
    )
    .unwrap();
    let error = format!(
        "{:#}",
        CheckedCache::open_and_verify(
            &candidate,
            "dolt.exe",
            EXE.len() as u64,
            &hex_digest(&Sha256::digest(EXE)),
            NOTICES.len() as u64,
            &hex_digest(&Sha256::digest(NOTICES)),
            &owned,
        )
        .err()
        .expect("a tampered notice is rejected")
    );
    assert!(error.contains("LICENSE-LLVM"), "{error}");

    // Missing, extra and resized notices are all rejected before publication.
    for (members, notices) in [
        (vec![("LICENSE-ICU", ICU_NOTICE)], &declared[..]),
        (
            vec![("LICENSE-ICU", ICU_NOTICE), ("LICENSE-LLVM", LLVM_NOTICE)],
            &declared[..1],
        ),
        (
            vec![
                ("LICENSE-ICU", ICU_NOTICE),
                ("LICENSE-LLVM", b"resized LLVM notice".as_slice()),
            ],
            &declared[..],
        ),
        (
            vec![("LICENSE-ICU", ICU_NOTICE), ("LICENSE-LLVM", LLVM_NOTICE)],
            &[][..],
        ),
    ] {
        let archive = built_zip(&members);
        let candidate = root.path().join(format!("rejected-{}", members.len()));
        let result = with_built_asset(&archive, notices, |asset| {
            extract(&archive, &candidate, asset)
        });
        assert!(
            result.is_err(),
            "{members:?} with {} declared",
            notices.len()
        );
        assert!(
            !candidate.join("dolt.exe").exists(),
            "no payload is published from a rejected inventory"
        );
    }
}
