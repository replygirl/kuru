#![cfg(feature = "tooling")]

use kuru_delivery::command::Command;
use serde_json::Value;
use std::{fs, path::Path, process::Output};

async fn cospec(root: &Path, args: &[&str]) -> Output {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        Command::new("cospec")
            .args(args)
            .current_dir(root)
            .env("BUN_OPTIONS", "--preload=cospec-preload.cjs")
            .env("NODE_PATH", root.join("support with spaces"))
            .env_remove("BUN_BE_BUN")
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("OPENSPEC_TELEMETRY", "0")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("standalone cospec timed out")
    .expect("run with mise so the pinned standalone cospec is available")
}

fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "cospec failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[tokio::test]
async fn standalone_cospec_emits_one_document_and_preserves_every_gate() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir(root.join("support with spaces")).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("support/cospec-preload.cjs"),
        root.join("support with spaces/cospec-preload.cjs"),
    )
    .unwrap();
    success(cospec(root, &["init", "--harness", "none", "--no-gate", "--yes"]).await);
    success(cospec(root, &["new", "docs", "gate-fixture"]).await);
    success(cospec(root, &["new", "docs", "dependency-fixture"]).await);
    let change = root.join("openspec/changes/gate-fixture");
    fs::write(
        change.join("proposal.md"),
        "## Why\n\nExercise the standalone gate before any implementation.\n\n\
         ## What Changes\n\nDocument one tested compatibility behavior.\n\n\
         ## Capabilities\n\nNo capability changes.\n\n\
         ## Impact\n\nDocumentation only.\n",
    )
    .unwrap();
    fs::write(
        change.join("tasks.md"),
        "## 1. Document\n\n- [ ] 1.1 Explain the observed compatibility behavior.\n",
    )
    .unwrap();
    let blockers = change.join("blocking-changes.md");
    let original = "# Dependencies\n\n## Blocked by\n\nNone.\n\n## Soft-blocked by\n\nNone.\n\n## Siblings\n\nNone.\n";
    fs::write(&blockers, original).unwrap();

    success(cospec(root, &["validate", "gate-fixture", "--strict"]).await);
    let output = success(cospec(root, &["apply", "gate-fixture", "--json"]).await);
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["gate"]["state"], "clear");
    assert_eq!(body["apply"]["progress"]["remaining"], 1);
    assert_eq!(body["apply"]["tasks"][0]["done"], false);

    let doctor = success(cospec(root, &["doctor", "--json"]).await);
    let doctor: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert!(
        doctor["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| {
                finding["check"] == "openspec-resolve"
                    && finding["message"]
                        .as_str()
                        .unwrap()
                        .contains("embedded pinned")
            })
    );

    for (heading, code, state) in [
        ("Blocked by", 2, "blocked"),
        ("Soft-blocked by", 3, "soft-blocked"),
    ] {
        fs::write(
            &blockers,
            original.replace(
                &format!("## {heading}\n\nNone."),
                &format!("## {heading}\n\n- [ ] `dependency-fixture` — required fixture"),
            ),
        )
        .unwrap();
        let output = cospec(root, &["apply", "gate-fixture", "--json"]).await;
        assert_eq!(output.status.code(), Some(code));
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["gate"]["state"], state);
        assert!(body.get("apply").is_none());
    }

    fs::write(&blockers, original).unwrap();
    fs::remove_file(change.join("tasks.md")).unwrap();
    let output = cospec(root, &["apply", "gate-fixture", "--json"]).await;
    assert_eq!(output.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["gate"]["reason"], "missing-artifacts");
    assert_eq!(body["gate"]["missingArtifacts"][0], "tasks");
}
