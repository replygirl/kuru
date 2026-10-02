#![cfg(feature = "tooling")]

use kuru_delivery::command::{self, Command};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write as _},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    process::Output,
    time::{Duration, Instant},
};

/// Hang guard for each standalone cospec call. It bounds a vendor CLI and is
/// not derived from a product budget; every call is bounded and attributed
/// separately, and none is retried.
const BOUND: Duration = Duration::from_secs(30);

/// One bounded call, kept with its label until the caller inspects it.
struct Call {
    label: String,
    elapsed: Duration,
    result: io::Result<Output>,
}

/// Run `command` under `bound` through the delivery command boundary. On
/// Windows its timeout error carries the phase, pipe EOFs, owned-tree
/// snapshot, cleanup result, CPU/working-set samples and output prefixes.
async fn run(label: &str, command: &mut Command, bound: Duration) -> Call {
    let started = Instant::now();
    let result = command::output(command, bound).await;
    Call {
        label: label.to_owned(),
        elapsed: started.elapsed(),
        result,
    }
}

/// Written to the raw stderr handle, which libtest does not capture, so
/// passing runs (coverage shards run without `--nocapture`) still record each
/// call's time in the job log.
fn report(line: &str) {
    let _ = writeln!(io::stderr(), "{line}");
}

impl Call {
    /// The process finished within its bound; its exit status is the caller's.
    #[track_caller]
    fn finished(self) -> Output {
        let elapsed = self.elapsed.as_millis();
        match self.result {
            Ok(output) => {
                report(&format!(
                    "cospec {}: {elapsed} ms (exit {:?})",
                    self.label,
                    output.status.code()
                ));
                output
            }
            Err(error) => {
                let hint = if error.kind() == io::ErrorKind::NotFound {
                    "; run with mise so the pinned standalone cospec is available"
                } else {
                    ""
                };
                panic!(
                    "standalone cospec {} failed after {elapsed} ms: {error}{hint}",
                    self.label
                )
            }
        }
    }

    /// The process finished within its bound and exited successfully.
    #[track_caller]
    fn succeeded(self) -> Output {
        let label = self.label.clone();
        let output = self.finished();
        assert!(
            output.status.success(),
            "standalone cospec {label} exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

async fn cospec(root: &Path, label: &str, args: &[&str]) -> Call {
    let mut command = Command::new("cospec");
    command
        .args(args)
        .current_dir(root)
        .env("BUN_OPTIONS", "--preload=cospec-preload.cjs")
        .env("NODE_PATH", root.join("support with spaces"))
        .env_remove("BUN_BE_BUN")
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("OPENSPEC_TELEMETRY", "0")
        .kill_on_drop(true);
    run(label, &mut command, BOUND).await
}

#[tokio::test]
async fn a_call_that_cannot_finish_names_its_label_in_the_failure() {
    let root = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    command
        .arg("never-finish")
        .current_dir(root.path())
        .kill_on_drop(true);
    let call = run("never-finish fixture", &mut command, Duration::from_secs(2)).await;
    let panic = catch_unwind(AssertUnwindSafe(|| call.finished()))
        .expect_err("a call past its bound must fail");
    let message = panic
        .downcast_ref::<String>()
        .expect("the failure is a formatted message");
    let mut required = vec![
        "standalone cospec never-finish fixture failed after ",
        "tool timed out",
    ];
    if cfg!(windows) {
        required.extend([
            "read native stdout/stderr after ",
            r#"arguments=["never-finish"]"#,
            "samples=[",
        ]);
    }
    for required in required {
        assert!(message.contains(required), "missing {required}: {message}");
    }
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
    cospec(
        root,
        "init",
        &["init", "--harness", "none", "--no-gate", "--yes"],
    )
    .await
    .succeeded();
    cospec(root, "new gate-fixture", &["new", "docs", "gate-fixture"])
        .await
        .succeeded();
    cospec(
        root,
        "new dependency-fixture",
        &["new", "docs", "dependency-fixture"],
    )
    .await
    .succeeded();
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

    cospec(root, "validate", &["validate", "gate-fixture", "--strict"])
        .await
        .succeeded();
    let output = cospec(root, "apply clear", &["apply", "gate-fixture", "--json"])
        .await
        .succeeded();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["gate"]["state"], "clear");
    assert_eq!(body["apply"]["progress"]["remaining"], 1);
    assert_eq!(body["apply"]["tasks"][0]["done"], false);

    let doctor = cospec(root, "doctor", &["doctor", "--json"])
        .await
        .succeeded();
    let doctor: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    let resolutions: Vec<&str> = doctor["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["check"] == "openspec-resolve")
        .map(|finding| finding["message"].as_str().unwrap())
        .collect();
    report(&format!("cospec doctor openspec-resolve: {resolutions:?}"));
    assert!(
        resolutions
            .iter()
            .any(|message| message.contains("embedded pinned")),
        "{resolutions:?}"
    );

    for (heading, code, state, label) in [
        ("Blocked by", 2, "blocked", "apply blocked"),
        ("Soft-blocked by", 3, "soft-blocked", "apply soft-blocked"),
    ] {
        fs::write(
            &blockers,
            original.replace(
                &format!("## {heading}\n\nNone."),
                &format!("## {heading}\n\n- [ ] `dependency-fixture` — required fixture"),
            ),
        )
        .unwrap();
        let output = cospec(root, label, &["apply", "gate-fixture", "--json"])
            .await
            .finished();
        assert_eq!(output.status.code(), Some(code));
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["gate"]["state"], state);
        assert!(body.get("apply").is_none());
    }

    fs::write(&blockers, original).unwrap();
    fs::remove_file(change.join("tasks.md")).unwrap();
    let output = cospec(
        root,
        "apply missing-tasks",
        &["apply", "gate-fixture", "--json"],
    )
    .await
    .finished();
    assert_eq!(output.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["gate"]["reason"], "missing-artifacts");
    assert_eq!(body["gate"]["missingArtifacts"][0], "tasks");
}
