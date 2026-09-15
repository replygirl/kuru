use super::*;
use kuru_archive::zip::{Limits, MemberKind, WriteMember, write};
use kuru_platform::fs::regular_file_info;
use std::io::{Seek, SeekFrom, Write};

const EXE: &[u8] = b"MZ fixture bytes, deliberately never executed";
const NOTICES: &[u8] = b"exact upstream notice fixture";

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
    let lock = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
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
    let lock = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("missing-runtime");
    let destination = cache.join("active");

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

#[cfg(windows)]
#[tokio::test]
async fn held_descendant_releases_after_checked_no_move_and_activation_recovers() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    private_directory(&cache).unwrap();
    let lock = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("runtime");
    extract(EMBEDDED_ARCHIVE, &candidate, BUNDLED_ASSET).unwrap();
    verify_version(
        &candidate.join(BUNDLED_ASSET.executable_name),
        &stage_path.join("probe"),
    )
    .await
    .unwrap();
    let source = files::directory(&candidate).unwrap();
    let source_identity = source.identity();
    // Even a delete-sharing data handle prevents moving its containing Windows
    // directory. The real probe already exited before this known blocker opens.
    let (_parent, blocker) = files::read(&candidate.join("LICENSES"), Privacy::OwnerOnly).unwrap();
    let destination = cache.join("active");
    let lock_path = cache.join(".install.lock");
    let mut blocker = Some(blocker);
    let mut denied = 0_u32;
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
    .await
    .unwrap();
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
    contender.try_lock().unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn persistent_held_descendant_exhausts_checked_recovery_and_preserves_stage() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    private_directory(&cache).unwrap();
    let lock = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("runtime");
    extract(EMBEDDED_ARCHIVE, &candidate, BUNDLED_ASSET).unwrap();
    verify_version(
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
    assert!(checked_denials > 1, "the checked denial must be retried");
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
    contender.try_lock().unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn cancelling_checked_activation_recovery_drops_stage_before_cache_lock() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache café 東京");
    private_directory(&cache).unwrap();
    let lock = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
    let lock_identity = regular_file_info(&lock).unwrap().identity;
    let stage = PrivateTemp::new(".install-", Some(&cache)).unwrap();
    let stage_path = stage.path().to_owned();
    let candidate = stage_path.join("runtime");
    extract(EMBEDDED_ARCHIVE, &candidate, BUNDLED_ASSET).unwrap();
    verify_version(
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
    contender.try_lock().unwrap();
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
    let first = cache_lock(root.path(), Duration::from_secs(1))
        .await
        .unwrap();
    let identity = regular_file_info(&first).unwrap().identity;
    assert!(cache_lock(root.path(), Duration::ZERO).await.is_err());
    drop(first);
    let second = cache_lock(root.path(), Duration::from_secs(1))
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
    let lock = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
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
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let parent = files::parent(&binary, Privacy::OwnerOnly, NameRetention::Movable).unwrap();
    let mut corrupt = parent.read_write(files::name(&binary).unwrap()).unwrap();
    corrupt.seek(SeekFrom::Start(0)).unwrap();
    corrupt.write_all(b"!").unwrap();
    corrupt.sync_all().unwrap();
    parent
        .verify(files::name(&binary).unwrap(), &corrupt)
        .unwrap();
    drop(corrupt);
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
    fs::remove_file(&binary).unwrap();
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
