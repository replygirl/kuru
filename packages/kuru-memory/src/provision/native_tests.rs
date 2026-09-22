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
        let (parent, file) = files::read(binary, Privacy::OwnerOnly)?;
        ensure!(
            regular_file_info(&file)?.identity == expected,
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
    let asset = crate::catalog::ASSETS
        .iter()
        .find(|asset| asset.target == "x86_64-pc-windows-msvc")
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
        .expect("prepare the required Windows archive through memory bundle:test-fixtures");
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
    assert_eq!(fs::read_dir(&candidate).unwrap().count(), 2);
    for (name, size, digest) in [
        ("dolt.exe", asset.executable_bytes, asset.executable_sha256),
        ("LICENSES", asset.license_bytes, asset.license_sha256),
    ] {
        let (_parent, file) = files::read(&candidate.join(name), Privacy::OwnerOnly).unwrap();
        kuru_platform::fs::require_private(&file).unwrap();
        assert_eq!(file.metadata().unwrap().len(), size);
        assert_eq!(
            hex_digest(&Sha256::digest(fs::read(candidate.join(name)).unwrap())),
            digest
        );
    }
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
    let detail = format!("{:#}", failure.cause);
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

    let (probe, (stage, lock)) = probe.probe((stage, lock)).await.unwrap();
    let destination = cache.join("active");
    let failure = activate_staged_after_probe(stage, lock, probe, &candidate, &destination)
        .await
        .unwrap()
        .expect("a published engine reports its retained stage instead of failing");
    assert_eq!(failure.stage, stage_container);
    let detail = format!("{:#}", failure.cause);
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
    assert_eq!(denied, 1, "release only the observed checked rejection");
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
    let (_parent, blocker) = files::read(&candidate.join("LICENSES"), Privacy::OwnerOnly).unwrap();
    let destination = cache.join("active");
    let lock_path = cache.join(".install.lock");
    let abort_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<tokio::task::AbortHandle>));
    let observer_abort_slot = abort_slot.clone();
    let task_destination = destination.clone();
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
