#[path = "../support/bundle.rs"]
mod bundle;

use bundle::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::{fs, path::Path};
#[cfg(windows)]
fn symlink(source: impl AsRef<Path>, target: impl AsRef<Path>) -> std::io::Result<()> {
    if source.as_ref().is_dir() {
        std::os::windows::fs::symlink_dir(source, target)
    } else {
        std::os::windows::fs::symlink_file(source, target)
    }
}

const MANIFEST: &[u8] = include_bytes!("../support/dolt-assets.json");

#[test]
fn target_selection_uses_requested_target_and_generates_coherent_versioned_payload_pins() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("support")
        .join("dolt-assets.json");
    let manifest = Manifest::load(&path).unwrap();
    for target in [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
    ] {
        let asset = manifest.select(target).unwrap();
        assert_eq!(asset.target, target);
        let generated = manifest.catalog(target).unwrap();
        let selected = generated
            .lines()
            .find(|line| line.contains("BUNDLED_ASSET"))
            .unwrap();
        assert!(selected.contains(target));
        assert!(selected.contains(&asset.archive_sha256));
        assert!(selected.contains(&asset.executable_sha256));
        assert!(selected.contains(&asset.license_sha256));
        assert!(generated.contains(&format!("DOLT_VERSION: &str = {:?}", manifest.version)));
    }
    for target in ["", "aarch64-pc-windows-msvc", "aarch64-unknown-linux-musl"] {
        assert!(
            manifest
                .select(target)
                .unwrap_err()
                .to_string()
                .contains("no host fallback")
        );
        assert!(manifest.catalog(target).is_err());
    }
}

#[test]
fn malformed_or_ambiguous_manifests_cannot_produce_a_bundle() {
    let valid: Value = serde_json::from_slice(MANIFEST).unwrap();
    for (pointer, replacement) in [
        ("/schema_version", json!(2)),
        ("/version", json!("2.03.3")),
        ("/version", json!("2.3.3/path")),
        ("/version", json!("2.3")),
        ("/upstream_commit", json!("bad")),
        ("/assets/0/target", json!("../escape")),
        ("/assets/0/stem", json!("dolt-linux-arm64")),
        ("/assets/0/format", json!("zip")),
        ("/assets/0/executable_name", json!("dolt.exe")),
        ("/assets/4/format", json!("tar.gz")),
        ("/assets/4/executable_name", json!("dolt")),
        ("/assets/4/expanded_bytes", json!(130339212)),
        ("/assets/0/url", json!("http://localhost/archive")),
        ("/assets/0/compressed_bytes", json!(MAX_COMPRESSED + 1)),
        ("/assets/0/compressed_bytes", json!(0)),
        ("/assets/0/expanded_bytes", json!(MAX_EXPANDED + 1)),
        ("/assets/0/license_bytes", json!(0)),
        ("/assets/0/executable_bytes", json!(u64::MAX)),
        ("/assets/0/archive_sha256", json!("bad")),
        ("/assets/0/executable_sha256", json!("F".repeat(64))),
        ("/assets/0/license_sha256", json!("0".repeat(63))),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            Manifest::parse(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut duplicate = valid.clone();
    duplicate["assets"][1] = duplicate["assets"][0].clone();
    assert!(Manifest::parse(&serde_json::to_vec(&duplicate).unwrap()).is_err());
    let mut missing = valid.clone();
    missing["assets"].as_array_mut().unwrap().pop();
    assert!(Manifest::parse(&serde_json::to_vec(&missing).unwrap()).is_err());
    let mut unknown = valid;
    unknown["unrecognized"] = json!(true);
    assert!(Manifest::parse(&serde_json::to_vec(&unknown).unwrap()).is_err());
    assert!(Manifest::parse(b"invalid").is_err());
    assert!(Manifest::parse(&vec![b' '; 65537]).is_err());
}

#[test]
fn prepared_inputs_reject_missing_truncated_corrupt_linked_and_mismatched_target_archives() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().canonicalize().unwrap();
    let mut manifest = Manifest::parse(MANIFEST).unwrap();
    let bytes = b"exact prepared compressed bytes";
    let asset = &mut manifest.assets[0];
    asset.compressed_bytes = bytes.len() as u64;
    asset.archive_sha256 = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = prepared_archive(&directory, asset);
    assert!(verified_archive(&path, asset).is_err());
    fs::write(&path, bytes).unwrap();
    assert_eq!(verified_archive(&path, asset).unwrap(), bytes);
    fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
    assert!(
        verified_archive(&path, asset)
            .unwrap_err()
            .to_string()
            .contains("size")
    );
    fs::write(&path, vec![b'!'; bytes.len()]).unwrap();
    assert!(
        verified_archive(&path, asset)
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    fs::write(&path, vec![b'!'; bytes.len() + 1]).unwrap();
    assert!(verified_archive(&path, asset).is_err());
    fs::write(&path, bytes).unwrap();
    let alias = directory.join("alias");
    symlink(&path, &alias).unwrap();
    assert!(verified_archive(&alias, asset).is_err());
    fs::remove_file(&alias).unwrap();
    fs::hard_link(&path, &alias).unwrap();
    assert!(verified_archive(&path, asset).is_err());
    fs::remove_file(&alias).unwrap();
    let linked_parent = directory.join("linked-parent");
    symlink(&directory, &linked_parent).unwrap();
    assert!(verified_archive(&linked_parent.join(path.file_name().unwrap()), asset).is_err());
    assert!(verified_archive(&directory, asset).is_err());
    #[cfg(unix)]
    {
        let fifo = directory.join("fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRUSR).unwrap();
        assert!(verified_archive(&fifo, asset).is_err());
    }
    assert!(verified_archive(Path::new("relative.archive"), asset).is_err());
    // PathBuf::push normalizes `..` when its Windows base is verbatim (as
    // canonicalize returns). Keep the actual hostile component in the input.
    let mut traversal = directory.as_os_str().to_owned();
    for component in [
        std::ffi::OsStr::new(".."),
        directory.file_name().unwrap(),
        path.file_name().unwrap(),
    ] {
        traversal.push(std::path::MAIN_SEPARATOR_STR);
        traversal.push(component);
    }
    let traversal = std::path::PathBuf::from(traversal);
    assert!(
        traversal
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    );
    assert!(verified_archive(&traversal, asset).is_err());
    asset.compressed_bytes = MAX_COMPRESSED + 1;
    assert!(verified_archive(&path, asset).is_err());
    assert!(
        verified_archive(&path, &manifest.assets[1]).is_err(),
        "a host archive cannot stand in for another target"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "rejected inputs are never rewritten"
    );
}

#[test]
fn build_mirror_is_explicit_absolute_and_independent_of_cargo_output_target() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let package = workspace.join("packages").join("kuru-memory");
    let mirror = root.path().join("offline mirror");
    assert_eq!(
        bundle_directory(&package, None).unwrap(),
        workspace.join("target").join("kuru-bundles")
    );
    assert_eq!(bundle_directory(&package, Some(&mirror)).unwrap(), mirror);
    assert!(bundle_directory(&package, Some(Path::new("relative"))).is_err());
    assert!(bundle_directory(Path::new("/"), None).is_err());
}

#[test]
fn default_build_mirror_reads_native_paths_and_distinguishes_missing_inputs() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("build workspace café 東京");
    let package = workspace.join("packages").join("kuru-memory");
    fs::create_dir_all(&package).unwrap();
    let directory = bundle_directory(&package, None).unwrap();
    let bytes = b"verified local build input";
    let mut manifest = Manifest::parse(MANIFEST).unwrap();
    let asset = &mut manifest.assets[0];
    asset.compressed_bytes = bytes.len() as u64;
    asset.archive_sha256 = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = prepared_archive(&directory, asset);

    let missing_directory = verified_archive(&path, asset).unwrap_err();
    assert_eq!(
        missing_directory
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(
        missing_directory
            .to_string()
            .contains("build-input directory")
    );

    fs::create_dir_all(&directory).unwrap();
    let missing_file = verified_archive(&path, asset).unwrap_err();
    assert_eq!(
        missing_file
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(missing_file.to_string().contains("build-input file"));

    fs::write(&path, bytes).unwrap();
    let original = fs::File::open(&path).unwrap();
    let original_identity = kuru_platform::fs::regular_file_info(&original)
        .unwrap()
        .identity;
    assert_eq!(verified_archive(&path, asset).unwrap(), bytes);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        kuru_platform::fs::regular_file_info(&fs::File::open(&path).unwrap())
            .unwrap()
            .identity,
        original_identity,
        "the verifier retains the prepared file instead of rewriting or replacing it"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn standard_macos_temporary_directory_aliases_remain_valid_build_locations() {
    let temporary = tempfile::Builder::new()
        .prefix("kuru-bundle-alias-")
        .tempdir_in("/tmp")
        .unwrap();
    let manifest_path = temporary.path().join("manifest.json");
    fs::write(&manifest_path, MANIFEST).unwrap();
    assert_eq!(Manifest::load(&manifest_path).unwrap().version, "2.3.3");
    let nested = temporary.path().join("nested-link");
    symlink(temporary.path(), &nested).unwrap();
    assert!(Manifest::load(&nested.join("manifest.json")).is_err());
}
