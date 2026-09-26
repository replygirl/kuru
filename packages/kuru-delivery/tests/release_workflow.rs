#![cfg(feature = "tooling")]
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use kuru_delivery::release::{self, GitHub, Version};
use kuru_delivery::{archive::digest, command, coverage::SHARDS, shell_support};
use kuru_platform::fs::make_executable;
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;

#[path = "support/repository_environment.rs"]
mod repository_environment;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
fn v(input: &str) -> Version {
    input.parse().unwrap()
}

#[test]
fn required_release_checks_precede_the_only_publication_job() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    let job = |name: &str| {
        workflow
            .split_once(&format!("\n  {name}:\n"))
            .unwrap_or_else(|| panic!("missing release job {name}"))
            .1
            .lines()
            .take_while(|line| !line.starts_with("  ") || line.starts_with("    "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // Without condition overrides, failed, cancelled, or skipped prerequisites
    // prevent their dependent jobs from running under GitHub's default policy.
    for (name, needs) in [
        ("build", "needs: [plan, bump, verify, verify-tests]"),
        ("assemble-candidate", "needs: [plan, bump, build, notes]"),
        (
            "verify-staged-windows",
            "needs: [plan, bump, assemble-candidate]",
        ),
        ("build-docs", "needs: [bump, assemble-candidate]"),
        ("deploy-docs", "needs: [build-docs, verify-staged-windows]"),
        (
            "publish",
            "needs: [plan, bump, assemble-candidate, verify-staged-windows, deploy-docs]",
        ),
    ] {
        let body = job(name);
        assert!(body.contains(needs), "{name} lost required dependencies");
        assert!(
            !body.lines().any(|line| line.starts_with("    if:")),
            "{name} must retain the default prerequisite success condition"
        );
        assert!(
            !body.contains("continue-on-error:"),
            "{name} must fail on unsuccessful required work"
        );
        if name != "build" {
            assert!(
                !body
                    .lines()
                    .any(|line| line.trim_start().starts_with("if:")),
                "{name} must not skip required steps"
            );
        }
    }
    let assembly = job("assemble-candidate");
    assert!(assembly.contains("mise run release:tool -- assemble"));
    assert!(assembly.contains("name=release-candidate-%s"));
    assert!(assembly.contains("RUN_ATTEMPT: ${{ github.run_attempt }}"));
    assert!(assembly.contains("artifact_name: ${{ steps.artifact.outputs.name }}"));
    assert!(assembly.contains("dist/*\n            RELEASE_NOTES.md"));
    assert!(assembly.contains("if-no-files-found: error"));
    assert!(!assembly.contains("contents: write"));
    assert!(!assembly.contains("GH_TOKEN:"));

    let build = job("build");
    for required in [
        "mise run //apps/kuru-tui:shell-support:generate",
        "mise run //packages/kuru-delivery:package:shell-support",
        "Get-FileHash -Algorithm SHA256",
        "openssl dgst -sha256",
    ] {
        assert!(build.contains(required), "native build lost {required}");
    }

    let verifier = job("verify-staged-windows");
    for required in [
        "runs-on: windows-2025",
        "ref: ${{ needs.bump.outputs.sha }}",
        "version: 2026.9.4",
        "name: ${{ needs.assemble-candidate.outputs.artifact_name }}",
        "path: candidate",
        "KURU_STAGED_WINDOWS_ARCHIVE: ${{ github.workspace }}/candidate/dist/kuru-${{ needs.plan.outputs.version }}-x86_64-pc-windows-msvc.zip",
        "mise run //apps/kuru-tui:verify:staged-windows",
    ] {
        assert!(verifier.contains(required), "missing {required}");
    }
    let execution = verifier
        .split("- name: Verify the exact staged package")
        .nth(1)
        .unwrap();
    assert!(!execution.contains("GITHUB_TOKEN"));
    assert!(!execution.contains("GH_TOKEN"));

    let publication = job("publish");
    assert!(publication.contains("ref: ${{ needs.bump.outputs.sha }}"));
    assert!(publication.contains("name: ${{ needs.assemble-candidate.outputs.artifact_name }}"));
    assert!(!publication.contains("pattern:"));
    assert_eq!(
        workflow.matches("mise run release:tool -- publish").count(),
        1
    );
    let published = job("verify-published-windows");
    for required in [
        "needs: [plan, bump, publish]",
        "runs-on: windows-2025",
        "contents: read",
        "ref: ${{ needs.bump.outputs.sha }}",
        "RELEASE_VERSION: ${{ needs.plan.outputs.version }}",
        "RELEASE_SHA: ${{ needs.bump.outputs.sha }}",
        "KURU_PUBLISHED_WINDOWS_RECEIPT: ${{ runner.temp }}/published-windows-receipt.json",
        "mise run //packages/kuru-delivery:verify:published-windows",
        "path: ${{ runner.temp }}/published-windows-receipt.json",
        "if-no-files-found: error",
    ] {
        assert!(published.contains(required), "missing {required}");
    }
    assert!(
        workflow.trim_end().ends_with("if-no-files-found: error"),
        "public verification receipt upload must be the final workflow step"
    );
}

#[test]
fn optional_published_windows_diagnostic_keeps_its_native_launcher() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task = fs::read_to_string(root.join("packages/kuru-delivery/mise.toml")).unwrap();
    let task = task
        .split("[tasks.\"verify:published-windows\"]")
        .nth(1)
        .unwrap();
    assert!(task.contains(
        "run_windows = \"pwsh.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File packages/kuru-delivery/support/verify-published-windows.ps1\""
    ));
    let script = fs::read_to_string(
        root.join("packages/kuru-delivery/support/verify-published-windows.ps1"),
    )
    .unwrap();
    let resolve = script
        .find("Get-Command mise -CommandType Application")
        .unwrap();
    let clear = script.find("verify-published-windows --mise").unwrap();
    assert!(
        resolve < clear,
        "native mise must be resolved before verifier env clearing"
    );
    assert!(!task.contains("kuru-memory"));
    assert!(!task.contains("kuru-tui"));
}

#[test]
fn native_workflow_shards_only_windows_and_keeps_the_aggregate_fail_closed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(root.join(".github/workflows/native-tests.yml")).unwrap();
    for required in [
        "if: inputs.os != 'windows-2025'",
        "windows-coverage:",
        "windows-coverage-report:",
        "windows-install:",
        "native-gate:",
        "needs: windows-coverage",
        "needs: [coverage, windows-coverage, windows-coverage-report, windows-install]",
        "test \"$KURU_NATIVE_WINDOWS_SHARDS\" = success",
        "test \"$KURU_NATIVE_WINDOWS_REPORT\" = success",
        "test \"$KURU_NATIVE_WINDOWS_INSTALL\" = success",
        "test \"$KURU_NATIVE_WINDOWS_INSTALL\" = skipped",
        "test \"$KURU_NATIVE_COVERAGE\" = skipped",
        "test \"$KURU_NATIVE_COVERAGE\" = success",
        "test \"$KURU_NATIVE_WINDOWS_SHARDS\" = skipped",
        "test \"$KURU_NATIVE_WINDOWS_REPORT\" = skipped",
        "delivery-archive",
        "kuru-delivery,kuru-archive",
        "application",
        "packages: kuru",
        "connectors-core-platform",
        "KURU_COVERAGE_TARGET",
        "KURU_COVERAGE_SOURCE",
        "KURU_COVERAGE_ATTEMPT",
        "KURU_COVERAGE_SHARD",
        "KURU_COVERAGE_PACKAGES",
        "KURU_COVERAGE_OUTPUT",
        "KURU_COVERAGE_INPUTS",
        "KURU_COVERAGE_REPORT",
        "KURU_COVERAGE_DIAGNOSTICS",
        "KURU_COVERAGE_JOB_STARTED=$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())",
        "coverage:windows:shard",
        "coverage:windows:collect",
        "cargo fetch --locked",
        "KURU_DOLT_BUNDLE_OFFLINE: \"true\"",
        "if-no-files-found: error",
    ] {
        assert!(workflow.contains(required), "missing {required}");
    }
    // Every shard restores one shared key; exactly one shard saves it.
    assert_eq!(
        workflow
            .matches("shared-key: native-coverage-windows\n")
            .count(),
        1
    );
    assert!(!workflow.contains("shared-key: native-coverage-windows-"));
    assert_eq!(
        workflow
            .matches("save-if: ${{ matrix.shard == 'connectors-core-platform' }}\n")
            .count(),
        1
    );
    assert_eq!(
        workflow.matches("actions/download-artifact@").count(),
        SHARDS.len()
    );
    assert_eq!(
        workflow
            .matches("actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c")
            .count(),
        SHARDS.len()
    );
    // Only a successful shard publishes receipt evidence under the name the
    // report accepts; failures publish diagnostics under a name it rejects.
    assert_eq!(workflow.matches("if: ${{ !cancelled() }}").count(), 1);
    let shards = workflow
        .split("\n  windows-coverage:\n")
        .nth(1)
        .unwrap()
        .split("\n  windows-coverage-report:\n")
        .next()
        .unwrap();
    assert!(!shards.contains("if: ${{ !cancelled() }}"));
    // The matrix rows are exactly the shards the receipt and collect code
    // enforce, in the same order, so the two cannot drift.
    let expected_matrix: String = SHARDS
        .iter()
        .map(|(shard, packages)| {
            format!(
                "          - shard: {shard}\n            packages: {}\n",
                packages.join(",")
            )
        })
        .collect();
    let matrix = shards
        .split("      matrix:\n        include:\n")
        .nth(1)
        .unwrap()
        .split("    steps:\n")
        .next()
        .unwrap();
    assert_eq!(matrix, expected_matrix);
    assert_eq!(shards.matches("if: ${{ failure() }}").count(), 1);
    assert!(shards.contains(
        "name: ${{ inputs.artifact-prefix }}-coverage-windows-${{ matrix.shard }}-diagnostics-attempt-${{ github.run_attempt }}"
    ));
    assert!(shards.contains(
        "name: ${{ inputs.artifact-prefix }}-coverage-windows-${{ matrix.shard }}-attempt-${{ github.run_attempt }}"
    ));
    // The job start is recorded before any other step, and the inner deadline
    // derives from the same limit the host enforces.
    assert!(
        shards
            .split("    steps:\n")
            .nth(1)
            .unwrap()
            .starts_with("      - name: Record the job start for the inner test deadline\n")
    );
    let limit = |prefix: &str, suffix: &str| {
        let lines: Vec<_> = shards
            .lines()
            .filter_map(|line| line.trim().strip_prefix(prefix))
            .map(|value| value.strip_suffix(suffix).unwrap().to_owned())
            .collect();
        assert_eq!(lines.len(), 1, "expected one {prefix}");
        lines[0].parse::<u64>().unwrap()
    };
    assert_eq!(
        limit("timeout-minutes: ", ""),
        limit("KURU_COVERAGE_JOB_MINUTES: \"", "\"")
    );
    // The report takes each shard's latest attempt from a pattern download.
    let report = workflow
        .split("\n  windows-coverage-report:\n")
        .nth(1)
        .unwrap()
        .split("\n  windows-install:\n")
        .next()
        .unwrap();
    for (shard, _) in SHARDS {
        assert!(report.contains(&format!(
            "          pattern: ${{{{ inputs.artifact-prefix }}}}-coverage-windows-{shard}-attempt-*\n          merge-multiple: false\n          path: ${{{{ runner.temp }}}}/kuru-coverage-inputs/{shard}\n"
        )));
    }
    assert!(!report.contains("-attempt-${{ github.run_attempt }}\n          path: ${{ runner.temp }}/kuru-coverage-inputs"));
    // An older successful attempt must not stand in for a shard whose latest
    // attempt failed: the report refuses before any other step unless every
    // shard job succeeded.
    let steps = report.split("    steps:\n").nth(1).unwrap();
    assert!(steps.starts_with(
        "      # Failed shards upload no receipt, so an older successful attempt would\n      # otherwise stand in for a shard whose latest attempt failed.\n      - name: Require every coverage shard job to have succeeded\n        shell: pwsh\n        env:\n          KURU_COVERAGE_SHARDS_RESULT: ${{ needs.windows-coverage.result }}\n        run: |\n          if ($env:KURU_COVERAGE_SHARDS_RESULT -ne 'success') {\n"
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn native_workflow_gate_rejects_incomplete_windows_results() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(root.join(".github/workflows/native-tests.yml")).unwrap();
    let gate = workflow
        .split("\n  native-gate:\n")
        .nth(1)
        .unwrap()
        .split("        run: |\n")
        .nth(1)
        .unwrap();
    async fn run(
        root: &Path,
        gate: &str,
        (os, install, coverage, shards, report, windows_install): (
            &str,
            &str,
            &str,
            &str,
            &str,
            &str,
        ),
    ) -> bool {
        let mut command = command::rooted(root, "bash");
        command
            .args(["-c", gate])
            .env("KURU_NATIVE_OS", os)
            .env("KURU_NATIVE_INSTALL", install)
            .env("KURU_NATIVE_COVERAGE", coverage)
            .env("KURU_NATIVE_WINDOWS_SHARDS", shards)
            .env("KURU_NATIVE_WINDOWS_REPORT", report)
            .env("KURU_NATIVE_WINDOWS_INSTALL", windows_install);
        command::bounded_output(&mut command, Duration::from_secs(5), 4096)
            .await
            .unwrap()
            .status
            .success()
    }

    assert!(
        run(
            &root,
            gate,
            (
                "windows-2025",
                "true",
                "skipped",
                "success",
                "success",
                "success"
            )
        )
        .await
    );
    assert!(
        run(
            &root,
            gate,
            (
                "windows-2025",
                "false",
                "skipped",
                "success",
                "success",
                "skipped"
            )
        )
        .await
    );
    assert!(
        run(
            &root,
            gate,
            (
                "ubuntu-24.04",
                "true",
                "success",
                "skipped",
                "skipped",
                "skipped"
            )
        )
        .await
    );
    for (label, shards, report, install) in [
        ("missing shard", "skipped", "success", "success"),
        ("failed shard", "failure", "success", "success"),
        ("cancelled shard", "cancelled", "success", "success"),
        ("missing report", "success", "skipped", "success"),
        ("failed report", "success", "failure", "success"),
        ("cancelled report", "success", "cancelled", "success"),
        ("missing install", "success", "success", "skipped"),
        ("failed install", "success", "success", "failure"),
        ("cancelled install", "success", "success", "cancelled"),
    ] {
        assert!(
            !run(
                &root,
                gate,
                ("windows-2025", "true", "skipped", shards, report, install),
            )
            .await,
            "native gate accepted {label}"
        );
    }
}
struct Repo {
    temp: TempDir,
}
impl Repo {
    async fn new() -> Self {
        let this = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        let root = this.root();
        release::git(root, &["init", "-b", "main"]).await.unwrap();
        for (key, value) in [
            ("user.name", "Release fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            release::git(root, &["config", key, value]).await.unwrap();
        }
        fs::create_dir(root.join("app")).unwrap();
        fs::write(
            root.join("app/Cargo.toml"),
            "[package]\nname = \"kuru\"\nversion.workspace = true\n",
        )
        .unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"app\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
        fs::write(root.join("Cargo.lock"), "version = 4\n\n[[package]]\nname = \"kuru\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.229\"\nsource = \"registry+https://example.invalid/index\"\nchecksum = \"unchanged\"\n").unwrap();
        fs::write(root.join("cog.toml"), include_str!("../../../cog.toml")).unwrap();
        fs::write(
            root.join("communique.toml"),
            include_str!("../../../communique.toml"),
        )
        .unwrap();
        this.commit("feat: initial peer harness").await;
        this
    }
    fn root(&self) -> &Path {
        self.temp.path()
    }
    async fn commit(&self, message: &str) -> String {
        release::git(self.root(), &["add", "."]).await.unwrap();
        release::git(self.root(), &["commit", "--allow-empty", "-m", message])
            .await
            .unwrap();
        self.head().await
    }
    async fn head(&self) -> String {
        release::git(self.root(), &["rev-parse", "HEAD"])
            .await
            .unwrap()
    }
    async fn tag(&self, tag: &str) {
        release::git(self.root(), &["tag", tag]).await.unwrap();
    }
}
#[tokio::test]
async fn hook_environment_cannot_redirect_rooted_commands() {
    if std::env::var_os(repository_environment::CHILD).is_some() {
        let repo = Repo::new().await;
        fs::write(
            repo.root().join("requested.txt"),
            "belongs to requested root",
        )
        .unwrap();
        repo.commit("fix: update the requested repository").await;
        assert_eq!(
            release::git(repo.root(), &["show", "HEAD:requested.txt"])
                .await
                .unwrap(),
            "belongs to requested root"
        );
        assert_eq!(
            release::git(
                repo.root(),
                &["config", "--default", "clean", "--get", "fixture.hook"]
            )
            .await
            .unwrap(),
            "clean"
        );
        assert_eq!(
            release::git(repo.root(), &["config", "--get", "user.signingkey"])
                .await
                .unwrap(),
            "fixture-signing-key"
        );
        release::run(
            repo.root(),
            env!("CARGO_BIN_EXE_kuru-delivery-fixture"),
            &["check-hook-env"],
        )
        .await
        .unwrap();
        return;
    }
    repository_environment::ForeignRepository::new()
        .assert_test_isolated("hook_environment_cannot_redirect_rooted_commands")
        .await;
}

#[tokio::test]
async fn hook_environment_preserves_versioning_and_private_recovery_index() {
    let foreign = repository_environment::ForeignRepository::new();
    for test in [
        "actual_cog_conventional_history_and_noop_are_read_only",
        "lost_bump_response_reuses_exact_tree_and_parent_even_after_main_advances",
    ] {
        foreign.assert_test_isolated(test).await;
    }
}

#[tokio::test]
async fn actual_cog_conventional_history_and_noop_are_read_only() {
    let repo = Repo::new().await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.1.0")
    );
    repo.tag("v0.1.0").await;
    assert!(
        release::compute_version(repo.root(), "auto")
            .await
            .unwrap_err()
            .to_string()
            .contains("no commits")
    );
    repo.commit("fix: preserve isolated memories").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.1.1")
    );
    repo.commit("feat: add peer relationships").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.2.0")
    );
    repo.commit("feat!: replace configuration format").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.2.0")
    );
    for (kind, expected) in [("major", "1.0.0"), ("minor", "0.2.0"), ("patch", "0.1.1")] {
        assert_eq!(
            release::compute_version(repo.root(), kind).await.unwrap(),
            v(expected)
        );
    }
    repo.tag("v1.0.0").await;
    repo.commit("fix!: remove legacy configuration").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("2.0.0")
    );
    assert_eq!(
        release::git(repo.root(), &["status", "--porcelain"])
            .await
            .unwrap(),
        ""
    );
    assert!(
        release::compute_version(repo.root(), "--help")
            .await
            .is_err()
    );
}
#[tokio::test]
async fn stamps_only_workspace_versions_and_rejects_inconsistent_locks() {
    let repo = Repo::new().await;
    let path = repo.root().join("Cargo.lock");
    let before: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(release::stamp(repo.root(), v("0.2.0")).unwrap().len(), 2);
    assert_eq!(
        release::workspace_version(repo.root(), None).await.unwrap(),
        v("0.2.0")
    );
    let after: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(before["package"][1], after["package"][1]);
    assert_eq!(after["package"][0]["version"].as_str(), Some("0.2.0"));
    assert!(release::stamp(repo.root(), v("0.2.0")).unwrap().is_empty());
    let manifest = fs::read(repo.root().join("Cargo.toml")).unwrap();
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("version = \"0.2.0\"", "version = \"0.0.1\""),
    )
    .unwrap();
    assert!(
        release::stamp(repo.root(), v("0.3.0"))
            .unwrap_err()
            .to_string()
            .contains("inconsistent")
    );
    assert_eq!(fs::read(repo.root().join("Cargo.toml")).unwrap(), manifest);
    fs::write(path, "version = 4\n").unwrap();
    assert!(
        release::stamp(repo.root(), v("0.3.0"))
            .unwrap_err()
            .to_string()
            .contains("every workspace")
    );
}
#[tokio::test]
async fn plans_initial_release_and_reuses_prepared_or_tagged_heads_without_inputs() {
    let repo = Repo::new().await;
    let head = repo.head().await;
    let initial = release::plan(repo.root(), "auto").await.unwrap();
    assert_eq!(initial.base_sha, head);
    assert_eq!(initial.version, "0.1.0");
    assert!(release::plan(repo.root(), "patch").await.is_err());
    repo.tag("v0.1.0").await;
    for strategy in ["auto", "major", "minor", "patch"] {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, head);
        assert_eq!(retry.version, "0.1.0");
    }
    repo.commit("feat: add relationships").await;
    let base = repo.head().await;
    let mut plans = Vec::new();
    for strategy in ["auto", "major", "minor", "patch"] {
        plans.push((
            strategy,
            release::plan(repo.root(), strategy).await.unwrap(),
        ));
    }
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let prepared = repo.commit("chore(release): v0.2.0").await;
    for strategy in ["auto", "major", "minor", "patch"] {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, prepared);
        assert_eq!(retry.version, "0.2.0");
    }
    repo.tag("v0.2.0").await;
    repo.commit("feat!: later work").await;
    repo.tag("v1.0.0").await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    for (strategy, original) in plans {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, original.base_sha);
        assert_eq!(retry.version, original.version);
    }
    assert!(release::plan(repo.root(), "invalid").await.is_err());
    fs::write(repo.root().join("untracked"), "dirty").unwrap();
    assert!(
        release::plan(repo.root(), "auto")
            .await
            .unwrap_err()
            .to_string()
            .contains("clean checkout")
    );
    for invalid in ["1.2", "01.2.3", "1.0.0\nx=y", "1.0.0;id", "1.0.0-rc.1", ""] {
        assert!(invalid.parse::<Version>().is_err());
    }
    for invalid in ["HEAD", "abcd", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"] {
        assert!(release::checked_sha(invalid).is_err());
    }
}

#[tokio::test]
async fn real_cli_calculates_plans_and_stamps_without_remote_credentials() {
    let repo = Repo::new().await;
    let binary = env!("CARGO_BIN_EXE_kuru-release");
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["version", "auto"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "0.1.0");
    let outputs = repo.root().join("outputs");
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["plan", "--bump", "auto"])
        .env("GITHUB_OUTPUT", &outputs)
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(
        fs::read_to_string(outputs)
            .unwrap()
            .contains("version=0.1.0\n")
    );
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["stamp", "0.2.0"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        release::workspace_version(repo.root(), None).await.unwrap(),
        v("0.2.0")
    );
    let output = kuru_delivery::command::Command::new(binary)
        .args(["commit", "--version", "0.2.0", "--expected-sha", A])
        .env_remove("GH_REPO")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GH_REPO is required"));
    let output = kuru_delivery::command::Command::new(binary)
        .args([
            "publish",
            "--version",
            "0.2.0",
            "--sha",
            A,
            "--notes",
            "missing",
        ])
        .env("GH_REPO", "fixture/kuru")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GH_TOKEN is required"));
    let output = kuru_delivery::command::Command::new(binary)
        .args(["stamp", "1.0.0;id"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
}

#[tokio::test]
async fn real_cli_assembles_a_candidate_without_github_credentials() {
    let archives = Archives::new();
    let output = kuru_delivery::command::Command::new(env!("CARGO_BIN_EXE_kuru-release"))
        .args([
            "assemble",
            "--version",
            "0.1.0",
            "--directory",
            archives.directory.to_str().unwrap(),
            "--notes",
            archives.notes.to_str().unwrap(),
        ])
        .env_remove("GH_REPO")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!(
            "Assembled {} release candidate checks",
            release::TARGETS.len() * 2 + 1
        )
    );
    assert_eq!(
        fs::read_to_string(archives.directory.join("SHA256SUMS"))
            .unwrap()
            .lines()
            .count(),
        release::TARGETS.len() * 2
    );
}

#[derive(Default)]
struct Remote {
    head: String,
    tag: Option<Value>,
    tag_object: Option<Value>,
    release: Option<Value>,
    calls: Vec<(Method, String, Value)>,
    uploads: Vec<String>,
    fail_upload: Option<usize>,
    corrupt_upload: bool,
    malformed: bool,
    prepared_commit: Option<Value>,
    comparison: Option<Value>,
    lose_commit_response: bool,
    manifest: Vec<u8>,
    manifest_redirect: Option<String>,
    lose_publish_response: bool,
}
struct Server {
    api: GitHub,
    state: Arc<Mutex<Remote>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(head: &str) -> Self {
        let state = Arc::new(Mutex::new(Remote {
            head: head.into(),
            ..Remote::default()
        }));
        let app = Router::new().fallback(handler).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            api: GitHub::with_endpoints("fixture/kuru", "fixture-token", &endpoint, &endpoint)
                .unwrap(),
            state,
            task,
        }
    }
}
fn response(status: StatusCode, value: Value) -> Response {
    (status, axum::Json(value)).into_response()
}
async fn handler(State(shared): State<Arc<Mutex<Remote>>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer fixture-token"
    );
    let bytes = to_bytes(request.into_body(), 300 * 1024 * 1024)
        .await
        .unwrap();
    let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut state = shared.lock().unwrap();
    state
        .calls
        .push((method.clone(), path.clone(), payload.clone()));
    if state.malformed {
        return (StatusCode::OK, "invalid-json").into_response();
    }
    let suffix = path.strip_prefix("/repos/fixture/kuru/").unwrap_or(&path);
    let value = match (method.clone(), suffix) {
        (Method::POST, "/graphql") => {
            if payload["variables"]["input"]["expectedHeadOid"].as_str() != Some(&state.head) {
                return response(
                    StatusCode::OK,
                    json!({"errors":[{"message":"expected head changed; private context"}]}),
                );
            }
            let sha = state
                .prepared_commit
                .as_ref()
                .map(|commit| commit["sha"].as_str().unwrap())
                .unwrap_or(B)
                .to_owned();
            state.head = sha.clone();
            if state.lose_commit_response {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"response lost after commit"}),
                );
            }
            json!({"data":{"createCommitOnBranch":{"commit":{"oid":sha}}}})
        }
        (Method::GET, "commits/main") => json!({"sha":state.head}),
        (Method::GET, value) if value.starts_with("compare/") => {
            assert_eq!(query, "per_page=1");
            state
                .comparison
                .clone()
                .unwrap_or(json!({"status":"diverged"}))
        }
        (Method::GET, value) if value.starts_with("git/ref/tags/") => {
            if let Some(tag) = &state.tag {
                json!({"object":tag})
            } else {
                return response(StatusCode::NOT_FOUND, json!({"message":"missing"}));
            }
        }
        (Method::POST, "git/tags") => {
            state.tag_object = Some(json!({"type":"commit","sha":payload["object"]}));
            json!({"sha":C})
        }
        (Method::POST, "git/refs") => {
            state.tag = Some(json!({"type":"tag","sha":payload["sha"]}));
            json!({"object":state.tag})
        }
        (Method::GET, value) if value.starts_with("git/tags/") => {
            json!({"object":state.tag_object})
        }
        (Method::GET, "releases") => state
            .release
            .as_ref()
            .map(|r| json!([r]))
            .unwrap_or(json!([])),
        (Method::POST, "releases") => {
            let mut value = payload;
            value["id"] = json!(17);
            value["assets"] = json!([]);
            value["html_url"] = json!("https://example.invalid/release");
            state.release = Some(value.clone());
            value
        }
        (Method::GET, "releases/17") => state.release.clone().unwrap(),
        (Method::POST, "releases/17/assets") => {
            if state.fail_upload == Some(state.uploads.len()) {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"fixture interrupted"}),
                );
            }
            let name = url::form_urlencoded::parse(query.as_bytes())
                .find(|(key, _)| key == "name")
                .unwrap()
                .1
                .into_owned();
            state.uploads.push(name.clone());
            let digest = if state.corrupt_upload {
                "0".repeat(64)
            } else {
                digest(&bytes)
            };
            let id = state.uploads.len() as u64;
            if name == "SHA256SUMS" {
                state.manifest = bytes.to_vec();
            }
            let asset = json!({"id":id,"name":name,"digest":format!("sha256:{digest}")});
            state.release.as_mut().unwrap()["assets"]
                .as_array_mut()
                .unwrap()
                .push(asset.clone());
            asset
        }
        (Method::GET, value) if value.starts_with("releases/assets/") => {
            if let Some(location) = &state.manifest_redirect {
                return (StatusCode::FOUND, [("location", location.clone())]).into_response();
            }
            return (StatusCode::OK, state.manifest.clone()).into_response();
        }
        (Method::PATCH, "releases/17") => {
            assert_eq!(
                payload["make_latest"], "legacy",
                "recovery must not force an older draft to become latest"
            );
            let release = state.release.as_mut().unwrap();
            release["draft"] = payload["draft"].clone();
            let result = release.clone();
            if state.lose_publish_response {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"response lost after publication"}),
                );
            }
            result
        }
        _ => {
            return response(
                StatusCode::BAD_REQUEST,
                json!({"message":"unexpected request"}),
            );
        }
    };
    response(StatusCode::OK, value)
}
#[tokio::test]
async fn signed_api_commit_uses_expected_head_and_only_scoped_payloads() {
    let repo = Repo::new().await;
    let head = repo.head().await;
    let server = Server::new(&head).await;
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.1.0"))
            .await
            .unwrap(),
        head
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
            .await
            .unwrap(),
        B
    );
    let payload = server
        .state
        .lock()
        .unwrap()
        .calls
        .iter()
        .find(|(_, path, _)| path == "/graphql")
        .unwrap()
        .2
        .clone();
    let input = &payload["variables"]["input"];
    assert_eq!(input["expectedHeadOid"], head);
    assert_eq!(input["branch"]["branchName"], "main");
    assert_eq!(input["message"]["headline"], "chore(release): v0.2.0");
    let files = input["fileChanges"]["additions"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let manifest = files.iter().find(|f| f["path"] == "Cargo.toml").unwrap();
    let data: toml::Value = toml::from_str(
        &String::from_utf8(
            STANDARD
                .decode(manifest["contents"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        data["workspace"]["package"]["version"].as_str(),
        Some("0.2.0")
    );
    server.state.lock().unwrap().head = C.into();
    let error = release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("diverged"));
    assert!(!error.contains("private context"));
    fs::write(repo.root().join("cog.toml"), "# unrelated change\n").unwrap();
    let requests_before = server.state.lock().unwrap().calls.len();
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
            .await
            .unwrap_err()
            .to_string(),
        "release stamp changed unrelated files: cog.toml",
    );
    assert_eq!(server.state.lock().unwrap().calls.len(), requests_before);
}
async fn comparison(repo: &Repo, base: &str, head: &str) -> Value {
    let commits = release::git(
        repo.root(),
        &["rev-list", "--reverse", &format!("{base}..{head}")],
    )
    .await
    .unwrap();
    let first = commits.lines().next().unwrap();
    let parents = release::git(repo.root(), &["show", "-s", "--format=%P", first])
        .await
        .unwrap();
    let tree = release::git(repo.root(), &["rev-parse", &format!("{first}^{{tree}}")])
        .await
        .unwrap();
    let message = release::git(repo.root(), &["show", "-s", "--format=%B", first])
        .await
        .unwrap();
    json!({"status":"ahead", "merge_base_commit":{"sha":base}, "commits":[{
        "sha":first, "parents":parents.split_whitespace().map(|sha| json!({"sha":sha})).collect::<Vec<_>>(),
        "commit":{"message":message, "tree":{"sha":tree}}
    }]})
}
#[tokio::test]
async fn lost_bump_response_reuses_exact_tree_and_parent_even_after_main_advances() {
    let repo = Repo::new().await;
    let base = repo.head().await;
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let prepared = repo.commit("chore(release): v0.2.0").await;
    let expected = comparison(&repo, &base, &prepared).await;
    fs::write(repo.root().join("later.txt"), "not part of this release").unwrap();
    let later = repo.commit("feat: later independent work").await;
    let later_comparison = comparison(&repo, &base, &later).await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let original_index = fs::read(repo.root().join(".git/index")).unwrap();
    let server = Server::new(&base).await;
    {
        let mut remote = server.state.lock().unwrap();
        remote.prepared_commit = Some(expected["commits"][0].clone());
        remote.comparison = Some(expected.clone());
        remote.lose_commit_response = true;
    }
    assert!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .is_err()
    );
    assert_eq!(server.state.lock().unwrap().head, prepared);
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .unwrap(),
        prepared
    );
    {
        let mut remote = server.state.lock().unwrap();
        remote.head = later;
        remote.comparison = Some(later_comparison);
    }
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .unwrap(),
        prepared
    );
    assert_eq!(
        fs::read(repo.root().join(".git/index")).unwrap(),
        original_index
    );
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, _, _)| method == Method::POST)
            .count(),
        1
    );

    // A title alone, or even identical contents with the wrong parent, is not
    // evidence that an intervening commit is the prepared release.
    for field in ["tree", "parent", "message", "merge", "diverged"] {
        let mut bad = expected.clone();
        match field {
            "tree" => bad["commits"][0]["commit"]["tree"]["sha"] = json!(C),
            "parent" => bad["commits"][0]["parents"][0]["sha"] = json!(C),
            "message" => bad["commits"][0]["commit"]["message"] = json!("feat: unrelated work"),
            "merge" => bad["commits"][0]["parents"]
                .as_array_mut()
                .unwrap()
                .push(json!({"sha":C})),
            _ => bad["status"] = json!("diverged"),
        }
        server.state.lock().unwrap().comparison = Some(bad);
        assert!(
            release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
                .await
                .is_err(),
            "accepted {field}"
        );
    }
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, _, _)| method == Method::POST)
            .count(),
        1
    );
}
#[tokio::test]
async fn unchanged_stamp_keeps_original_ancestor_without_including_new_main_work() {
    let repo = Repo::new().await;
    let base = repo.head().await;
    fs::write(repo.root().join("later.txt"), "later").unwrap();
    let later = repo.commit("fix: later work").await;
    let ahead = comparison(&repo, &base, &later).await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    let server = Server::new(&later).await;
    server.state.lock().unwrap().comparison = Some(ahead);
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.1.0"))
            .await
            .unwrap(),
        base
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    server.state.lock().unwrap().comparison.as_mut().unwrap()["merge_base_commit"]["sha"] =
        json!(C);
    assert!(
        release::commit_version(repo.root(), &server.api, &base, v("0.1.0"))
            .await
            .is_err()
    );
}
struct Archives {
    temp: TempDir,
    directory: PathBuf,
    notes: PathBuf,
}
impl Archives {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("dist");
        fs::create_dir(&directory).unwrap();
        // The candidate verifier decodes every exact core and support inventory.
        let binary = temp.path().join("fixture-kuru");
        fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        make_executable(&File::open(&binary).unwrap()).unwrap();
        let generated = temp.path().join("generated");
        fs::create_dir_all(generated.join("completions")).unwrap();
        fs::create_dir_all(generated.join("man")).unwrap();
        for name in shell_support::NAMES {
            fs::write(generated.join(name), format!("fixture {name}\n")).unwrap();
        }
        for target in release::TARGETS {
            kuru_delivery::archive::package(&binary, target, "0.1.0", &directory).unwrap();
            shell_support::package(&generated, target, "0.1.0", &directory).unwrap();
        }
        let notes = temp.path().join("notes.md");
        fs::write(&notes, "# Kuru 0.1.0\n\nPersistent peer conversations.\n").unwrap();
        release::assemble(&directory, v("0.1.0"), &notes).unwrap();
        Self {
            temp,
            directory,
            notes,
        }
    }
    async fn publish(&self, api: &GitHub) -> anyhow::Result<String> {
        release::publish(api, &self.directory, v("0.1.0"), A, &self.notes).await
    }
}
#[tokio::test]
async fn interrupted_draft_resumes_missing_assets_then_publishes_exact_commit() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().fail_upload = Some(2);
    assert!(archives.publish(&server.api).await.is_err());
    {
        let remote = server.state.lock().unwrap();
        assert_eq!(remote.release.as_ref().unwrap()["draft"], true);
        assert!(
            remote
                .calls
                .iter()
                .all(|(method, _, _)| method != Method::PATCH)
        );
    }
    server.state.lock().unwrap().fail_upload = None;
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert_eq!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap()
            .as_deref(),
        Some(A)
    );
    {
        let remote = server.state.lock().unwrap();
        assert_eq!(remote.uploads.len(), release::TARGETS.len() * 2 + 1);
        assert_eq!(remote.release.as_ref().unwrap()["draft"], false);
        assert!(
            remote.release.as_ref().unwrap()["body"]
                .as_str()
                .unwrap()
                .contains(A)
        );
    }
    let snapshot = server.state.lock().unwrap().release.clone();
    server.state.lock().unwrap().calls.clear();
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert_eq!(server.state.lock().unwrap().release, snapshot);
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    assert_eq!(
        fs::read_to_string(archives.directory.join("SHA256SUMS"))
            .unwrap()
            .lines()
            .count(),
        release::TARGETS.len() * 2
    );
}
#[tokio::test]
async fn lost_publication_response_recovers_from_remote_checksums_without_rebuilding() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().lose_publish_response = true;
    assert!(archives.publish(&server.api).await.is_err());
    assert_eq!(
        server.state.lock().unwrap().release.as_ref().unwrap()["draft"],
        false
    );
    fs::remove_dir_all(&archives.directory).unwrap();
    fs::remove_file(&archives.notes).unwrap();
    server.state.lock().unwrap().calls.clear();
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
}
#[tokio::test]
async fn published_recovery_rejects_incomplete_or_mismatched_remote_content() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    archives.publish(&server.api).await.unwrap();
    let original = server.state.lock().unwrap().release.clone().unwrap();
    let original_manifest = server.state.lock().unwrap().manifest.clone();
    for invalid in [
        "asset",
        "digest",
        "marker",
        "manifest",
        "oversize",
        "redirect",
        "missing-tag",
    ] {
        {
            let mut remote = server.state.lock().unwrap();
            remote.release = Some(original.clone());
            remote.manifest = original_manifest.clone();
            remote.calls.clear();
            remote.manifest_redirect = None;
            match invalid {
                "asset" => {
                    remote.release.as_mut().unwrap()["assets"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                "digest" => {
                    remote.release.as_mut().unwrap()["assets"][1]["digest"] =
                        json!(format!("sha256:{}", "0".repeat(64)))
                }
                "marker" => remote.release.as_mut().unwrap()["body"] = json!("unrelated release"),
                "manifest" => remote.manifest = b"inconsistent checksum data".to_vec(),
                "oversize" => remote.manifest = vec![b'x'; 4097],
                "redirect" => {
                    remote.manifest_redirect = Some("http://example.invalid/unsafe".into())
                }
                _ => remote.tag = None,
            }
        }
        assert!(
            archives.publish(&server.api).await.is_err(),
            "accepted {invalid}"
        );
        assert!(
            server
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .all(|(method, _, _)| method == Method::GET)
        );
    }
}
#[tokio::test]
async fn missing_corrupt_assets_and_empty_notes_prevent_all_remote_writes() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    let support_name = shell_support::archive_name("0.1.0", release::TARGETS[0]).unwrap();
    let support_path = archives.directory.join(&support_name);
    let original_support = fs::read(&support_path).unwrap();
    fs::remove_file(&support_path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    fs::write(&support_path, b"not a support envelope").unwrap();
    fs::write(
        archives.directory.join(format!("{support_name}.sha256")),
        format!("{}  {support_name}\n", digest(b"not a support envelope")),
    )
    .unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes).is_err(),
        "candidate accepted a checksummed but invalid support envelope"
    );
    fs::write(&support_path, &original_support).unwrap();
    fs::write(
        archives.directory.join(format!("{support_name}.sha256")),
        format!("{}  {support_name}\n", digest(&original_support)),
    )
    .unwrap();
    let name = kuru_delivery::archive::archive_name("0.1.0", release::TARGETS[0]).unwrap();
    let path = archives.directory.join(name);
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    fs::write(&path, b"corrupt").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    fs::write(&path, &original).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, &original).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(256 * 1024 * 1024 + 1)
        .unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("bounded regular")
    );
    fs::write(path, original).unwrap();
    fs::write(archives.directory.join("unexpected.sha256"), "unexpected\n").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    fs::remove_file(archives.directory.join("unexpected.sha256")).unwrap();
    fs::write(archives.directory.join("SHA256SUMS"), "stale\n").unwrap();
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("candidate checksum manifest")
    );
    release::assemble(&archives.directory, v("0.1.0"), &archives.notes).unwrap();
    fs::write(&archives.notes, "").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("nonempty")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("nonempty")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
}
#[tokio::test]
async fn immutable_tag_conflicts_and_unowned_or_corrupt_drafts_are_rejected() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().tag = Some(json!({"type":"commit","sha":B}));
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("different commit")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    server.state.lock().unwrap().tag = None;
    server.state.lock().unwrap().fail_upload = Some(1);
    assert!(archives.publish(&server.api).await.is_err());
    server.state.lock().unwrap().release.as_mut().unwrap()["assets"][0]["digest"] = json!("bad");
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    server.state.lock().unwrap().release.as_mut().unwrap()["body"] = json!("another draft");
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("does not belong")
    );
}
#[tokio::test]
async fn bad_api_shapes_and_final_uploaded_digest_never_publish() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().corrupt_upload = true;
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method != Method::PATCH)
    );
    server.state.lock().unwrap().malformed = true;
    assert!(release::tag_commit(&server.api, v("0.1.0")).await.is_err());
    server.state.lock().unwrap().malformed = false;
    server.state.lock().unwrap().tag = Some(json!({"type":"tree","sha":C}));
    assert!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("resolve")
    );
    server.state.lock().unwrap().tag = Some(json!({"type":"tag","sha":C}));
    server.state.lock().unwrap().tag_object = Some(json!({"type":"tag","sha":C}));
    assert!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("resolve")
    );
    assert!(GitHub::new("bad/repo/path", "fixture").is_err());
    assert!(
        GitHub::with_endpoints(
            "fixture/kuru",
            "fixture",
            "http://example.invalid/",
            "http://example.invalid/"
        )
        .is_err()
    );
    assert!(GitHub::new("fixture/kuru", "").is_err());
    assert!(archives.temp.path().exists());
}
