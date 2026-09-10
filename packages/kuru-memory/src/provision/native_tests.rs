use super::*;
use kuru_archive::zip::{Limits, MemberKind, WriteMember, write};
use kuru_platform::fs::regular_file_info;

const EXE: &[u8] = b"MZ fixture bytes, deliberately never executed";
const NOTICES: &[u8] = b"exact upstream notice fixture";

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
                .join("target/kuru-bundles")
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
