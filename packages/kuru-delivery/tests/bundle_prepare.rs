#![cfg(feature = "tooling")]

use kuru_delivery::command::Command;
use kuru_delivery::{
    archive::digest,
    bundle::{self, PrepareOptions},
};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::PathBuf,
};
#[path = "support/files.rs"]
mod files;
use files::{identity, mode, symlink};

const TARGET: &str = "aarch64-apple-darwin";
const OTHER: &str = "x86_64-unknown-linux-gnu";

struct Fixture {
    root: tempfile::TempDir,
    manifest: PathBuf,
    input: PathBuf,
    cache: PathBuf,
    bytes: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let manifest = root.path().join("manifest.json");
        let input = root.path().join("input.archive");
        let cache = root.path().join("bundles");
        let bytes = b"verified target archive fixture; never executable".to_vec();
        fs::write(&input, &bytes).unwrap();
        fs::write(
            &manifest,
            serde_json::to_vec(&json!({
                "schema_version": 2, "version": "2.3.3", "upstream_commit": "a".repeat(40),
                "assets": [asset(TARGET, &bytes), asset(OTHER, b"different target archive")],
            }))
            .unwrap(),
        )
        .unwrap();
        Self {
            root,
            manifest,
            input,
            cache,
            bytes,
        }
    }
    fn options(&self) -> PrepareOptions {
        PrepareOptions {
            manifest: self.manifest.clone(),
            target: TARGET.into(),
            bundle_dir: self.cache.clone(),
            archive: Some(self.input.clone()),
            offline: true,
        }
    }
    fn output(&self) -> PathBuf {
        self.cache.join(format!("{}.archive", digest(&self.bytes)))
    }
    fn mutate(&self, edit: impl FnOnce(&mut Value)) {
        let mut manifest = serde_json::from_slice(&fs::read(&self.manifest).unwrap()).unwrap();
        edit(&mut manifest);
        fs::write(&self.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }
    fn clean_staging(&self) {
        if !self.cache.exists() {
            return;
        }
        for entry in fs::read_dir(&self.cache).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(
                !name.to_string_lossy().starts_with(".bundle-"),
                "staging leaked: {name:?}"
            );
        }
    }
}
fn asset(target: &str, bytes: &[u8]) -> Value {
    json!({ "target": target, "stem": "dolt-fixture", "format": "tar.gz", "executable_name": "dolt", "provenance": "upstream", "url": "https://example.invalid/pinned.tar.gz",
        "compressed_bytes": bytes.len(), "archive_sha256": digest(bytes),
        "expanded_bytes": 100, "executable_bytes": 60, "executable_sha256": "b".repeat(64),
        "license_bytes": 20, "license_sha256": "c".repeat(64) })
}
fn error(result: anyhow::Result<PathBuf>) -> String {
    format!("{:#}", result.unwrap_err())
}
async fn output(command: &mut Command) -> std::process::Output {
    kuru_delivery::command::output(command, std::time::Duration::from_secs(15))
        .await
        .expect("bundle CLI exceeded fixture deadline or failed to execute")
}

#[tokio::test]
async fn local_import_is_private_exact_and_valid_cache_is_reused_offline() {
    let fixture = Fixture::new();
    let prepared = bundle::prepare(&fixture.options()).await.unwrap();
    assert_eq!(fs::read(&prepared).unwrap(), fixture.bytes);
    assert_eq!(prepared.file_name(), fixture.output().file_name());
    kuru_platform::fs::require_private(&File::open(&prepared).unwrap()).unwrap();
    #[cfg(windows)]
    assert!(
        File::options().write(true).open(&prepared).is_err(),
        "published build input must retain its sealed read-only DACL"
    );
    kuru_platform::fs::Directory::open(
        &fixture.cache,
        kuru_platform::fs::Privacy::OwnerOnly,
        kuru_platform::fs::NameRetention::Movable,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&prepared).unwrap().permissions().mode() & 0o777,
            0o400
        );
        assert_eq!(
            fs::metadata(&fixture.cache).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let before = identity(&prepared);
    fs::remove_file(&fixture.input).unwrap();
    let mut options = fixture.options();
    options.archive = None;
    assert_eq!(bundle::prepare(&options).await.unwrap(), prepared);
    assert_eq!(before, identity(&prepared));
    fixture.clean_staging();
}

#[tokio::test]
async fn missing_offline_and_unknown_target_fail_without_host_fallback() {
    let fixture = Fixture::new();
    let mut options = fixture.options();
    options.archive = None;
    assert!(error(bundle::prepare(&options).await).contains("missing in offline mode"));
    assert!(!fixture.output().exists());
    options.target = "future-unknown-target".into();
    assert!(error(bundle::prepare(&options).await).contains("host fallback is disabled"));
    fixture.clean_staging();
}

#[tokio::test]
async fn explicit_target_selects_its_own_bytes_independently_of_helper_host() {
    let fixture = Fixture::new();
    let bytes = b"different target archive";
    fs::write(&fixture.input, bytes).unwrap();
    let mut options = fixture.options();
    options.target = OTHER.into();
    let output = bundle::prepare(&options).await.unwrap();
    assert_eq!(
        output.file_name().unwrap(),
        format!("{}.archive", digest(bytes)).as_str()
    );
    assert_eq!(fs::read(output).unwrap(), bytes);
    assert!(!fixture.output().exists());
}

#[tokio::test]
async fn corrupt_or_truncated_input_never_publishes_and_preserves_existing_cache() {
    let fixture = Fixture::new();
    let mut corrupt = fixture.bytes.clone();
    corrupt[0] ^= 1;
    fs::write(&fixture.input, &corrupt).unwrap();
    assert!(error(bundle::prepare(&fixture.options()).await).contains("checksum mismatch"));
    assert!(!fixture.output().exists());
    fixture.clean_staging();
    fs::write(&fixture.input, b"short").unwrap();
    assert!(error(bundle::prepare(&fixture.options()).await).contains("size mismatch"));
    assert!(!fixture.output().exists());
    fixture.clean_staging();
    fs::write(&fixture.input, &fixture.bytes).unwrap();
    bundle::prepare(&fixture.options()).await.unwrap();
    mode(&fixture.output(), 0o600);
    fs::write(fixture.output(), &corrupt).unwrap();
    let before = identity(&fixture.output());
    assert!(error(bundle::prepare(&fixture.options()).await).contains("checksum mismatch"));
    assert_eq!(before, identity(&fixture.output()));
    assert_eq!(fs::read(fixture.output()).unwrap(), corrupt);
    assert_eq!(fs::read(&fixture.input).unwrap(), fixture.bytes);
    fixture.clean_staging();
}

#[tokio::test]
async fn concurrent_cli_imports_converge_on_one_verified_file_and_stable_lock() {
    let fixture = Fixture::new();
    let spawn = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
        command
            .args([
                "bundle",
                "prepare",
                "--offline",
                "--target",
                TARGET,
                "--manifest",
            ])
            .arg(&fixture.manifest)
            .arg("--bundle-dir")
            .arg(&fixture.cache)
            .arg("--archive")
            .arg(&fixture.input)
            .env_remove("KURU_DOLT_BUNDLE_DIR")
            .kill_on_drop(true);
        command
    };
    let mut first = spawn();
    let mut second = spawn();
    let (first, second) = tokio::join!(output(&mut first), output(&mut second));
    assert!(
        first.status.success() && second.status.success(),
        "first: {}\nsecond: {}\ncache: {:?}",
        String::from_utf8_lossy(&first.stderr),
        String::from_utf8_lossy(&second.stderr),
        fs::read_dir(&fixture.cache).map(|entries| entries
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>())
    );
    for result in [first, second] {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(fs::read(fixture.output()).unwrap(), fixture.bytes);
    let lock = fixture.cache.join(".prepare.lock");
    let before = identity(&lock);
    let held = File::options().read(true).write(true).open(&lock).unwrap();
    held.try_lock().unwrap();
    drop(held);
    bundle::prepare(&fixture.options()).await.unwrap();
    assert_eq!(before, identity(&lock));
    fixture.clean_staging();
}

#[tokio::test]
async fn unsafe_source_manifest_and_cache_objects_are_preserved() {
    for object in [
        "archive-symlink",
        "archive-hardlink",
        "manifest-symlink",
        "cache-symlink",
        "cache-public",
        "lock-hardlink",
        "output-symlink",
        "output-directory",
        "output-public",
    ] {
        let fixture = Fixture::new();
        let original = fixture.root.path().join("original");
        fs::write(&original, &fixture.bytes).unwrap();
        match object {
            "archive-symlink" => {
                fs::remove_file(&fixture.input).unwrap();
                symlink(&original, &fixture.input).unwrap();
            }
            "archive-hardlink" => {
                fs::remove_file(&fixture.input).unwrap();
                fs::hard_link(&original, &fixture.input).unwrap();
            }
            "manifest-symlink" => {
                fs::rename(&fixture.manifest, fixture.root.path().join("real-manifest")).unwrap();
                symlink(fixture.root.path().join("real-manifest"), &fixture.manifest).unwrap();
            }
            "cache-symlink" => {
                fs::create_dir(fixture.root.path().join("foreign")).unwrap();
                symlink(fixture.root.path().join("foreign"), &fixture.cache).unwrap();
            }
            "cache-public" => {
                fs::create_dir(&fixture.cache).unwrap();
                mode(&fixture.cache, 0o755);
            }
            _ => {
                fs::create_dir(&fixture.cache).unwrap();
                mode(&fixture.cache, 0o700);
                match object {
                    "lock-hardlink" => {
                        fs::hard_link(&original, fixture.cache.join(".prepare.lock")).unwrap()
                    }
                    "output-symlink" => symlink(&original, fixture.output()).unwrap(),
                    "output-directory" => fs::create_dir(fixture.output()).unwrap(),
                    "output-public" => {
                        fs::write(fixture.output(), &fixture.bytes).unwrap();
                        mode(&fixture.output(), 0o644);
                    }
                    _ => unreachable!(),
                }
            }
        }
        assert!(
            bundle::prepare(&fixture.options()).await.is_err(),
            "accepted {object}"
        );
        assert_eq!(fs::read(&original).unwrap(), fixture.bytes);
        fixture.clean_staging();
    }
}

#[tokio::test]
async fn manifest_rejects_ambiguous_unbounded_and_network_unsafe_inputs() {
    for case in [
        "schema",
        "duplicate",
        "hash",
        "zero",
        "oversized",
        "http",
        "credentials",
        "query",
        "payload",
        "unknown",
        "provenance",
        "unpinned-upstream",
        "notices-upstream",
    ] {
        let fixture = Fixture::new();
        fixture.mutate(|value| match case {
            "schema" => value["schema_version"] = json!(1),
            "duplicate" => value["assets"][1]["target"] = json!(TARGET),
            "hash" => value["assets"][0]["archive_sha256"] = json!("../unsafe"),
            "zero" => value["assets"][0]["compressed_bytes"] = json!(0),
            "oversized" => value["assets"][0]["compressed_bytes"] = json!(64 * 1024 * 1024 + 1),
            "http" => value["assets"][0]["url"] = json!("http://127.0.0.1:1/archive"),
            "credentials" => {
                value["assets"][0]["url"] = json!("https://fake:fake@example.invalid/archive")
            }
            "query" => {
                value["assets"][0]["url"] = json!("https://example.invalid/archive?mutable=true")
            }
            "payload" => value["assets"][0]["license_bytes"] = json!(u64::MAX),
            "unknown" => value["unexpected"] = json!(true),
            "provenance" => {
                value["assets"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("provenance");
            }
            "unpinned-upstream" => {
                value["assets"][0]["archive_sha256"] = json!("unpinned");
                value["assets"][0]["executable_sha256"] = json!("unpinned");
                for size in ["compressed_bytes", "expanded_bytes", "executable_bytes"] {
                    value["assets"][0][size] = Value::Null;
                }
            }
            "notices-upstream" => value["assets"][0]["notices"] = json!([]),
            _ => unreachable!(),
        });
        assert!(
            bundle::prepare(&fixture.options()).await.is_err(),
            "accepted {case}"
        );
        assert!(!fixture.cache.exists(), "{case} touched the cache");
    }
    let fixture = Fixture::new();
    fs::write(&fixture.manifest, vec![b' '; 65537]).unwrap();
    assert!(error(bundle::prepare(&fixture.options()).await).contains("exceeds 64 KiB"));
}

#[tokio::test]
async fn cli_directory_precedence_and_conventional_default_are_explicit() {
    let fixture = Fixture::new();
    let override_dir = fixture.root.path().join("explicit");
    let environment_dir = fixture.root.path().join("environment");
    let mut child = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    child
        .args([
            "bundle",
            "prepare",
            "--offline",
            "--target",
            TARGET,
            "--manifest",
        ])
        .arg(&fixture.manifest)
        .arg("--archive")
        .arg(&fixture.input)
        .arg("--bundle-dir")
        .arg(&override_dir)
        .env("KURU_DOLT_BUNDLE_DIR", &environment_dir)
        .env("KURU_DOLT_BUNDLE_TARGET", OTHER)
        .env(
            "KURU_DOLT_BUNDLE_ARCHIVE",
            fixture.root.path().join("absent"),
        )
        .kill_on_drop(true);
    let result = output(&mut child).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        override_dir
            .join(format!("{}.archive", digest(&fixture.bytes)))
            .exists()
    );
    assert!(!environment_dir.exists());
    let mut child = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    child
        .args(["bundle", "prepare", "--manifest"])
        .arg(&fixture.manifest)
        .env("KURU_DOLT_BUNDLE_DIR", &environment_dir)
        .env("KURU_DOLT_BUNDLE_TARGET", TARGET)
        .env("KURU_DOLT_BUNDLE_ARCHIVE", &fixture.input)
        .env("KURU_DOLT_BUNDLE_OFFLINE", "true")
        .kill_on_drop(true);
    let result = output(&mut child).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read(environment_dir.join(format!("{}.archive", digest(&fixture.bytes)))).unwrap(),
        fixture.bytes
    );
    assert!(bundle::bundle_directory(&fixture.manifest, Some("relative".into())).is_err());
    assert!(bundle::bundle_directory(&fixture.manifest, None).is_err());
    let standard = fixture
        .root
        .path()
        .join("packages/kuru-memory/support/dolt-assets.json");
    assert!(
        bundle::bundle_directory(&standard, None)
            .unwrap()
            .ends_with("target/kuru-bundles")
    );
}

#[tokio::test]
async fn host_sentinel_selects_only_the_declared_host_archive() {
    let fixture = Fixture::new();
    fixture.mutate(|manifest| {
        manifest["assets"] = json!([asset(
            kuru_delivery::archive::host_target().unwrap(),
            &fixture.bytes
        )]);
    });
    let mut options = fixture.options();
    options.target = "host".into();
    let output = bundle::prepare(&options).await.unwrap();
    assert_eq!(fs::read(output).unwrap(), fixture.bytes);
}

const BUILT: &str = "aarch64-pc-windows-msvc";

/// The committed built entry, re-pinned to fixture bytes whose license and
/// notice sizes satisfy the built zip rule against this fixture's upstreams.
fn built_asset(bytes: &[u8], pinned: bool) -> Value {
    let committed: Value =
        serde_json::from_str(include_str!("../../kuru-memory/support/dolt-assets.json")).unwrap();
    let mut asset = committed["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["target"] == BUILT)
        .unwrap()
        .clone();
    asset["license_bytes"] = json!(20);
    asset["license_sha256"] = json!("c".repeat(64));
    asset["build"]["sources"]["dolt"]["version"] =
        json!(format!("v0.40.5-0.20260909181817-{}", "a".repeat(12)));
    if pinned {
        for notice in asset["notices"].as_array_mut().unwrap() {
            notice["bytes"] = json!(10);
            notice["sha256"] = json!("d".repeat(64));
        }
        asset["compressed_bytes"] = json!(bytes.len());
        asset["archive_sha256"] = json!(digest(bytes));
        asset["executable_bytes"] = json!(60);
        asset["executable_sha256"] = json!("b".repeat(64));
        asset["expanded_bytes"] = json!(60 + 20 + 30);
    }
    asset
}

#[tokio::test]
async fn unpinned_built_entry_never_affects_upstream_preparation() {
    let fixture = Fixture::new();
    fixture.mutate(|manifest| {
        manifest["assets"]
            .as_array_mut()
            .unwrap()
            .push(built_asset(b"", false));
    });
    // Upstream preparation imports and reuses exactly as before.
    let prepared = bundle::prepare(&fixture.options()).await.unwrap();
    assert_eq!(fs::read(&prepared).unwrap(), fixture.bytes);
    let mut options = fixture.options();
    options.archive = None;
    assert_eq!(bundle::prepare(&options).await.unwrap(), prepared);
    // Selecting the unpinned built target refuses with the pinning instruction
    // before creating or reading anything.
    let cache = fixture.root.path().join("built-cache");
    let mut built = fixture.options();
    built.target = BUILT.into();
    built.bundle_dir = cache.clone();
    let message = error(bundle::prepare(&built).await);
    assert!(
        message.contains(
            "Dolt engine for aarch64-pc-windows-msvc is built from source and not yet pinned: run `mise run //packages/kuru-memory:bundle:build -- --target aarch64-pc-windows-msvc --print-pins` on linux-x64 and commit the pins"
        ),
        "{message}"
    );
    assert!(!cache.exists());
    fixture.clean_staging();
}

#[tokio::test]
async fn pinned_built_entry_is_imported_but_never_downloaded() {
    let fixture = Fixture::new();
    let bytes = b"built windows arm64 archive fixture".to_vec();
    fixture.mutate(|manifest| {
        manifest["assets"]
            .as_array_mut()
            .unwrap()
            .push(built_asset(&bytes, true));
    });
    let mut options = fixture.options();
    options.target = BUILT.into();
    options.archive = None;
    options.offline = false;
    let message = error(bundle::prepare(&options).await);
    assert!(
        message.contains("is built from source: run `mise run //packages/kuru-memory:bundle:build -- --target aarch64-pc-windows-msvc` on linux-x64, then import it with --archive"),
        "{message}"
    );
    assert!(!fixture.cache.exists() || fs::read_dir(&fixture.cache).unwrap().count() == 1);
    let input = fixture.root.path().join("built.zip");
    fs::write(&input, &bytes).unwrap();
    options.archive = Some(input);
    options.offline = true;
    let prepared = bundle::prepare(&options).await.unwrap();
    assert_eq!(fs::read(&prepared).unwrap(), bytes);
    options.archive = None;
    assert_eq!(bundle::prepare(&options).await.unwrap(), prepared);
    fixture.clean_staging();
}

#[tokio::test]
async fn built_entry_rules_are_enforced_by_the_independent_preparer() {
    for (case, expected) in [
        ("url", "must not declare a URL"),
        ("no-build", "declare their build"),
        ("recipe", "unknown bundle build recipe"),
        ("host", "only on linux-x64"),
        ("platform", "platform does not match"),
        ("tags", "tags do not match"),
        ("module", "unexpected Dolt Go module"),
        ("pseudo", "does not pin the upstream commit"),
        ("sum", "invalid Dolt module checksum"),
        ("icu", "invalid ICU source pin"),
        ("go", "invalid Go toolchain pin"),
        ("llvm", "invalid llvm-mingw toolchain pin"),
        ("license", "upstream Godeps/LICENSES"),
        ("mixed", "all pinned or all unpinned"),
        ("no-notices", "third-party notices"),
        ("notice-name", "notice name"),
        ("notice-path", "notice path"),
        ("notice-pin", "invalid bundle notice pin"),
        ("notice-half", "both pinned or both unpinned"),
        ("pinned-notice", "requires pinned notices"),
        ("expanded", "invalid bounded payload sizes"),
        ("target", "no bundle source build is defined"),
    ] {
        let fixture = Fixture::new();
        fixture.mutate(|manifest| {
            let pinned = matches!(case, "pinned-notice" | "expanded");
            let mut built = built_asset(b"built", pinned);
            match case {
                "url" => built["url"] = json!("https://example.invalid/built.zip"),
                "no-build" => {
                    built.as_object_mut().unwrap().remove("build");
                }
                "recipe" => built["build"]["recipe"] = json!("docker/1"),
                "host" => built["build"]["host"] = json!("macos-arm64"),
                "platform" => built["build"]["goarch"] = json!("amd64"),
                "tags" => built["build"]["tags"] = json!(["timetzdata"]),
                "module" => {
                    built["build"]["sources"]["dolt"]["module"] = json!("example.test/dolt")
                }
                "pseudo" => {
                    built["build"]["sources"]["dolt"]["version"] =
                        json!("v0.40.5-0.20260909181817-79caf258c32e")
                }
                "sum" => built["build"]["sources"]["dolt"]["sum"] = json!("sha256:abc"),
                "icu" => built["build"]["sources"]["icu"]["bytes"] = json!(0),
                "go" => {
                    built["build"]["toolchain"]["go"]["url"] = json!("https://example.test/go.tgz")
                }
                "llvm" => built["build"]["toolchain"]["llvm_mingw"]["sha256"] = json!("x"),
                "license" => built["license_sha256"] = json!("e".repeat(64)),
                "mixed" => built["archive_sha256"] = json!("f".repeat(64)),
                "no-notices" => built["notices"] = json!([]),
                "notice-name" => built["notices"][1]["name"] = json!("LICENSES"),
                "notice-path" => built["notices"][1]["path"] = json!("../escape"),
                "notice-pin" => built["notices"][0]["sha256"] = json!("F".repeat(64)),
                "notice-half" => built["notices"][0]["bytes"] = Value::Null,
                "pinned-notice" => {
                    built["notices"][0]["bytes"] = Value::Null;
                    built["notices"][0]["sha256"] = json!("unpinned");
                }
                "expanded" => built["expanded_bytes"] = json!(60 + 20),
                "target" => built["target"] = json!("x86_64-pc-windows-gnu"),
                _ => unreachable!(),
            }
            manifest["assets"].as_array_mut().unwrap().push(built);
        });
        let message = error(bundle::prepare(&fixture.options()).await);
        assert!(message.contains(expected), "{case}: {message}");
        assert!(!fixture.cache.exists(), "{case} touched the cache");
    }
}
