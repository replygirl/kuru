#![cfg(feature = "tooling")]

use kuru_delivery::command::Command;
use std::path::Path;

async fn run(configure: impl FnOnce(&mut Command)) -> (bool, String) {
    let root = tempfile::tempdir().unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("kuru-memory/support/dolt-assets.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    command
        .args(["bundle", "build", "--manifest"])
        .arg(manifest)
        .arg("--work-dir")
        .arg(root.path().join("work"))
        .arg("--output")
        .arg(root.path().join("out"))
        .env_remove("KURU_DOLT_BUNDLE_OFFLINE")
        .env_remove("KURU_BUNDLE_BUILD_HOST_OVERRIDE")
        .kill_on_drop(true);
    configure(&mut command);
    let output = kuru_delivery::command::output(&mut command, std::time::Duration::from_secs(15))
        .await
        .expect("bundle build CLI exceeded fixture deadline or failed to execute");
    assert!(!root.path().join("work").exists(), "refusal created work");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[tokio::test]
async fn cli_refuses_offline_builds_before_any_work() {
    let (success, stderr) = run(|command| {
        command
            .args(["--target", "aarch64-pc-windows-msvc", "--print-pins"])
            .env("KURU_DOLT_BUNDLE_OFFLINE", "true");
    })
    .await;
    assert!(!success);
    assert!(
        stderr.contains("unset KURU_DOLT_BUNDLE_OFFLINE"),
        "{stderr}"
    );
}

#[tokio::test]
async fn cli_requires_print_pins_for_the_committed_unpinned_entry() {
    // The override passes the host gate on every test host; the committed
    // arm64 entry is still unpinned, so the build stops before any work.
    let (success, stderr) = run(|command| {
        command
            .args(["--target", "aarch64-pc-windows-msvc"])
            .env("KURU_BUNDLE_BUILD_HOST_OVERRIDE", "1");
    })
    .await;
    assert!(!success);
    assert!(
        stderr.contains("aarch64-pc-windows-msvc is built from source and not yet pinned"),
        "{stderr}"
    );
    let (success, stderr) = run(|command| {
        command
            .args(["--target", "x86_64-pc-windows-msvc", "--print-pins"])
            .env("KURU_BUNDLE_BUILD_HOST_OVERRIDE", "1");
    })
    .await;
    assert!(!success);
    assert!(
        stderr.contains("uses the upstream Dolt archive"),
        "{stderr}"
    );
}

#[test]
fn manifest_toolchain_pins_equal_the_memory_package_lockfile() {
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(packages.join("kuru-memory/support/dolt-assets.json")).unwrap(),
    )
    .unwrap();
    let lock: toml::Value =
        toml::from_str(&std::fs::read_to_string(packages.join("kuru-memory/mise.lock")).unwrap())
            .unwrap();
    let built = manifest["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["provenance"] == "built")
        .unwrap();
    for (tool, pinned) in [
        ("go", &built["build"]["toolchain"]["go"]),
        (
            "github:mstorsjo/llvm-mingw",
            &built["build"]["toolchain"]["llvm_mingw"],
        ),
    ] {
        let entries = lock["tools"][tool].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{tool}");
        let entry = &entries[0];
        let version = pinned["version"].as_str().unwrap();
        assert_eq!(
            entry["version"].as_str().unwrap(),
            version.trim_start_matches("go"),
            "{tool}"
        );
        let platform = &entry["platforms.linux-x64"];
        assert_eq!(
            platform["url"].as_str().unwrap(),
            pinned["url"].as_str().unwrap(),
            "{tool}"
        );
        assert_eq!(
            platform["checksum"].as_str().unwrap(),
            format!("sha256:{}", pinned["sha256"].as_str().unwrap()),
            "{tool}"
        );
    }
}
