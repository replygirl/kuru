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
        assert_eq!(asset.provenance, Provenance::Upstream);
        assert!(asset.notices().is_empty());
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
        assert!(selected.contains("notices: &[] }"));
        assert!(generated.contains(&format!("DOLT_VERSION: &str = {:?}", manifest.version)));
    }
    for target in ["", "aarch64-unknown-linux-musl"] {
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
        ("/schema_version", json!(1)),
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
    asset.compressed_bytes = Some(bytes.len() as u64);
    asset.archive_sha256 = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = prepared_archive(&directory, asset).unwrap();
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
    asset.compressed_bytes = Some(MAX_COMPRESSED + 1);
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
    asset.compressed_bytes = Some(bytes.len() as u64);
    asset.archive_sha256 = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = prepared_archive(&directory, asset).unwrap();

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

const BUILT: usize = 5;

fn with(valid: &Value, pointer: &str, replacement: Value) -> Value {
    let mut changed = valid.clone();
    match changed.pointer_mut(pointer) {
        Some(slot) => *slot = replacement,
        None => {
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            changed.pointer_mut(parent).unwrap()[key] = replacement;
        }
    }
    changed
}

fn without(valid: &Value, pointer: &str) -> Value {
    let mut changed = valid.clone();
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    changed
        .pointer_mut(parent)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove(key)
        .unwrap();
    changed
}

fn parse_error(value: &Value) -> String {
    Manifest::parse(&serde_json::to_vec(value).unwrap())
        .map(|_| "accepted".to_owned())
        .unwrap_or_else(|error| format!("{error:#}"))
}

/// Pin the built entry with synthetic values that satisfy the zip size rule.
fn pinned_built(valid: &Value) -> Value {
    let mut pinned = valid.clone();
    let asset = &mut pinned["assets"][BUILT];
    for (index, size) in [(1, 15141), (2, 12155)] {
        asset["notices"][index]["bytes"] = json!(size);
        asset["notices"][index]["sha256"] = json!("d".repeat(64));
    }
    let notices: u64 = asset["notices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|notice| notice["bytes"].as_u64().unwrap())
        .sum();
    asset["compressed_bytes"] = json!(40_000_000);
    asset["archive_sha256"] = json!("c".repeat(64));
    asset["executable_bytes"] = json!(118_142_976);
    asset["executable_sha256"] = json!("e".repeat(64));
    asset["expanded_bytes"] = json!(118_142_976 + 1_212_811 + notices);
    pinned
}

#[test]
fn committed_manifest_is_schema_two_with_one_unpinned_built_entry() {
    let manifest = Manifest::parse(MANIFEST).unwrap();
    assert_eq!(manifest.schema_version, 2);
    let built: Vec<_> = manifest
        .assets
        .iter()
        .filter(|asset| asset.provenance == Provenance::Built)
        .collect();
    assert_eq!(built.len(), 1);
    let asset = built[0];
    assert_eq!(asset.target, "aarch64-pc-windows-msvc");
    assert_eq!(asset.stem, "dolt-windows-arm64");
    assert!(asset.url.is_none());
    let build = asset.build.as_ref().unwrap();
    assert_eq!(build.host, "linux-x64");
    assert_eq!(build.sources.icu.version, "78.3");
    assert_eq!(build.toolchain.go.version, "go1.26.2");
    assert_eq!(build.toolchain.llvm_mingw.version, "20260922");
    let names: Vec<_> = asset.notices().iter().map(|notice| &*notice.name).collect();
    assert_eq!(
        names,
        ["LICENSE-ICU", "LICENSE-LLVM", "LICENSE-MINGW-W64-RUNTIME"]
    );
    assert_eq!(asset.notices()[0].from, NoticeSource::Icu);
    assert_eq!(asset.notices()[1].from, NoticeSource::LlvmMingw);
    for upstream in manifest
        .assets
        .iter()
        .filter(|asset| asset.provenance == Provenance::Upstream)
    {
        assert!(upstream.build.is_none() && upstream.notices.is_none());
        assert!(upstream.pins().is_ok());
    }
}

#[test]
fn unpinned_built_asset_is_refused_as_an_input_without_affecting_other_targets() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().canonicalize().unwrap();
    let manifest = Manifest::parse(MANIFEST).unwrap();
    let asset = manifest.select("aarch64-pc-windows-msvc").unwrap();
    let instruction = "not yet pinned: run `mise run //packages/kuru-memory:bundle:build -- --target aarch64-pc-windows-msvc --print-pins` on linux-x64 and commit the pins";
    for error in [
        asset.pins().unwrap_err(),
        prepared_archive(&directory, asset).unwrap_err(),
        verified_archive(&directory.join("any.archive"), asset).unwrap_err(),
        manifest.catalog("aarch64-pc-windows-msvc").unwrap_err(),
    ] {
        assert!(error.to_string().contains(instruction), "{error}");
    }
    // The runtime catalog for every upstream target lists only pinned assets.
    let generated = manifest.catalog("x86_64-pc-windows-msvc").unwrap();
    assert!(generated.contains("pub(crate) const ASSETS: [Asset<'static>; 5]"));
    assert!(!generated.contains("aarch64-pc-windows-msvc"));
    assert!(!generated.contains("unpinned"));
}

#[test]
fn pinned_built_asset_is_catalogued_with_its_notices() {
    let valid: Value = serde_json::from_slice(MANIFEST).unwrap();
    let pinned = pinned_built(&valid);
    let manifest = Manifest::parse(&serde_json::to_vec(&pinned).unwrap()).unwrap();
    let asset = manifest.select("aarch64-pc-windows-msvc").unwrap();
    assert_eq!(asset.pins().unwrap().compressed_bytes, 40_000_000);
    let generated = manifest.catalog("aarch64-pc-windows-msvc").unwrap();
    let selected = generated
        .lines()
        .find(|line| line.contains("BUNDLED_ASSET"))
        .unwrap();
    assert!(selected.contains("dolt-windows-arm64"));
    assert!(selected.contains(
        "Notice { name: \"LICENSE-ICU\", bytes: 27718, sha256: \"e55522d81edc687a341a4411e0776e54ca654e90147f354a90458aaced4116af\" }"
    ));
    assert!(generated.contains("pub(crate) const ASSETS: [Asset<'static>; 6]"));
    // Notices count toward the exact zip expansion only for built assets.
    let resized = with(
        &pinned,
        "/assets/5/expanded_bytes",
        json!(118_142_976 + 1_212_811),
    );
    assert!(parse_error(&resized).contains("payload sizes"));
    // A pinned archive cannot carry unpinned notices.
    let partial = with(&pinned, "/assets/5/notices/1/sha256", json!("unpinned"));
    let partial = with(&partial, "/assets/5/notices/1/bytes", Value::Null);
    assert!(parse_error(&partial).contains("requires pinned notices"));
    for (pointer, replacement) in [
        ("/assets/5/compressed_bytes", json!(MAX_COMPRESSED + 1)),
        ("/assets/5/expanded_bytes", json!(MAX_EXPANDED + 1)),
    ] {
        assert!(
            parse_error(&with(&pinned, pointer, replacement)).contains("exceeds budget"),
            "{pointer}"
        );
    }
}

#[test]
fn schema_two_provenance_build_and_notice_rules_fail_closed() {
    let valid: Value = serde_json::from_slice(MANIFEST).unwrap();
    let upstream_url = valid["assets"][4]["url"].clone();
    let build = valid["assets"][BUILT]["build"].clone();
    let notices = valid["assets"][BUILT]["notices"].clone();
    let cases = [
        (without(&valid, "/assets/0/provenance"), "provenance"),
        (
            with(&valid, "/assets/0/provenance", json!("mirror")),
            "unknown variant",
        ),
        (
            with(&valid, "/assets/5/url", upstream_url),
            "must not declare a URL",
        ),
        (
            with(&valid, "/assets/0/build", build),
            "must not declare a build or notices",
        ),
        (
            with(&valid, "/assets/0/notices", notices),
            "must not declare a build or notices",
        ),
        (
            with(&valid, "/assets/0/provenance", json!("built")),
            "must not declare a URL",
        ),
        (
            with(&valid, "/assets/5/provenance", json!("upstream")),
            "can never be unpinned",
        ),
        (without(&valid, "/assets/5/build"), "declare their build"),
        (
            with(&valid, "/assets/5/build/recipe", json!("dolt-docker/1")),
            "unknown Dolt build recipe",
        ),
        (
            with(&valid, "/assets/5/build/host", json!("macos-arm64")),
            "only on linux-x64",
        ),
        (
            with(&valid, "/assets/5/build/goarch", json!("amd64")),
            "platform does not match",
        ),
        (
            with(&valid, "/assets/5/build/tags", json!(["icu_static"])),
            "tags do not match",
        ),
        (
            with(
                &valid,
                "/assets/5/build/sources/dolt/module",
                json!("github.com/dolthub/dolt"),
            ),
            "unexpected Dolt Go module",
        ),
        (
            with(
                &valid,
                "/assets/5/build/sources/dolt/version",
                json!("v0.40.5-0.20260909181817-000000000000"),
            ),
            "does not pin the upstream commit",
        ),
        (
            with(
                &valid,
                "/assets/5/build/sources/dolt/sum",
                json!("h1:short="),
            ),
            "invalid Dolt module checksum",
        ),
        (
            with(
                &valid,
                "/assets/5/build/sources/icu/sha256",
                json!("A".repeat(64)),
            ),
            "invalid ICU source pin",
        ),
        (
            with(
                &valid,
                "/assets/5/build/sources/icu/url",
                json!("http://example.test/icu.tgz"),
            ),
            "invalid ICU source pin",
        ),
        (
            with(
                &valid,
                "/assets/5/build/toolchain/go/version",
                json!("1.26.2"),
            ),
            "invalid Go toolchain pin",
        ),
        (
            with(
                &valid,
                "/assets/5/build/toolchain/llvm_mingw/url",
                json!("https://example.test/llvm-mingw.tar.xz"),
            ),
            "invalid llvm-mingw toolchain pin",
        ),
        (
            with(&valid, "/assets/5/build/extra", json!(true)),
            "unknown field",
        ),
        (
            with(&valid, "/assets/5/license_sha256", json!("f".repeat(64))),
            "upstream Godeps/LICENSES",
        ),
        (
            with(&valid, "/assets/0/archive_sha256", json!("unpinned")),
            "all pinned or all unpinned",
        ),
        (
            with(&valid, "/assets/5/compressed_bytes", json!(1)),
            "all pinned or all unpinned",
        ),
        (
            with(&valid, "/assets/5/notices", json!([])),
            "third-party notices",
        ),
        (without(&valid, "/assets/5/notices"), "third-party notices"),
        (
            with(&valid, "/assets/5/notices/1/name", json!("LICENSE-ICU")),
            "notice name",
        ),
        (
            with(&valid, "/assets/5/notices/1/name", json!("LICENSES")),
            "notice name",
        ),
        (
            with(&valid, "/assets/5/notices/1/name", json!("LICENSE-lower")),
            "notice name",
        ),
        (
            with(&valid, "/assets/5/notices/1/from", json!("go")),
            "unknown variant",
        ),
        (
            with(&valid, "/assets/5/notices/1/path", json!("../LICENSE.TXT")),
            "notice path",
        ),
        (
            with(&valid, "/assets/5/notices/1/path", json!("/LICENSE.TXT")),
            "notice path",
        ),
        (
            with(&valid, "/assets/5/notices/1/path", json!("bin\\LICENSE")),
            "notice path",
        ),
        (
            with(&valid, "/assets/5/notices/0/path", json!("LICENSE")),
            "notice path",
        ),
        (
            with(&valid, "/assets/5/notices/1/bytes", json!(5)),
            "invalid Dolt notice pin",
        ),
        (
            with(&valid, "/assets/5/notices/0/sha256", json!("unpinned")),
            "invalid Dolt notice pin",
        ),
        (
            with(&valid, "/assets/5/notices/0/bytes", Value::Null),
            "both pinned or both unpinned",
        ),
        (
            with(
                &valid,
                "/assets/5/target",
                json!("aarch64-unknown-linux-musl"),
            ),
            "unsupported Dolt bundle target",
        ),
    ];
    for (manifest, expected) in cases {
        let error = parse_error(&manifest);
        assert!(error.contains(expected), "{expected}: {error}");
    }
}
