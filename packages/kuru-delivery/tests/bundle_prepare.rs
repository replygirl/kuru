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
                "schema_version": 1, "version": "2.3.3", "upstream_commit": "a".repeat(40),
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
    json!({ "target": target, "stem": "dolt-fixture", "format": "tar.gz", "executable_name": "dolt", "url": "https://example.invalid/pinned.tar.gz",
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
    ] {
        let fixture = Fixture::new();
        fixture.mutate(|value| match case {
            "schema" => value["schema_version"] = json!(2),
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
