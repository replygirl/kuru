#![cfg(feature = "tooling")]

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use kuru_delivery::command;

const ORIGIN: &str = "https://github.com/RustSec/advisory-db.git";
const FETCH_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

async fn git(directory: &Path, arguments: &[&str]) {
    let mut command = command::rooted(directory, "git");
    command.args(arguments);
    let started = Instant::now();
    let output = command::bounded_output(&mut command, Duration::from_secs(10), 4096)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "git {arguments:?} in {directory:?} failed after {:?}: {error}",
                started.elapsed()
            )
        });
    assert!(
        output.status.success(),
        "git {arguments:?} in {directory:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn git_with_environment(directory: &Path, arguments: &[&str], environment: &[(&str, &str)]) {
    let mut command = command::rooted(directory, "git");
    command.args(arguments).envs(environment.iter().copied());
    let started = Instant::now();
    let output = command::bounded_output(&mut command, Duration::from_secs(10), 4096)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "git {arguments:?} in {directory:?} failed after {:?}: {error}",
                started.elapsed()
            )
        });
    assert!(
        output.status.success(),
        "git {arguments:?} in {directory:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn advisory_database(root: &Path) -> PathBuf {
    let database = root.join("advisory-db");
    fs::create_dir(&database).unwrap();
    git(&database, &["init"]).await;
    fs::write(database.join("README.md"), b"fixture\n").unwrap();
    git(&database, &["add", "README.md"]).await;
    git(
        &database,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "fixture",
        ],
    )
    .await;
    git(&database, &["remote", "add", "origin", ORIGIN]).await;
    git(&database, &["config", "remote.origin.fetch", FETCH_REFSPEC]).await;
    git(&database, &["checkout", "--detach", "HEAD"]).await;
    database
}

async fn scan_cli(
    database: &Path,
    marker: &Path,
    advance_head: bool,
    fail: bool,
    cargo_home: Option<&Path>,
) -> std::process::Output {
    let mut command = command::Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("KURU_ADVISORY_DB", database)
        .env("KURU_AUDIT_CAPTURE", marker)
        .args(["audit", "scan", "--audit-bin"])
        .arg(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    if advance_head {
        command.env("KURU_AUDIT_ADVANCE_HEAD", "1");
    }
    if fail {
        command.env("KURU_AUDIT_FAIL", "1");
    }
    if let Some(cargo_home) = cargo_home {
        command.env("CARGO_HOME", cargo_home);
    }
    command::bounded_output(&mut command, Duration::from_secs(30), 64 * 1024)
        .await
        .unwrap()
}

#[tokio::test]
async fn cli_uses_direct_audit_binary_with_root_lockfile_and_offline_flags() {
    let root = tempfile::tempdir().unwrap();
    let database = advisory_database(root.path()).await;
    let marker = root.path().join("audit-arguments.json");
    let mut command = command::Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("KURU_ADVISORY_DB", &database)
        .env("KURU_AUDIT_CAPTURE", &marker)
        .args(["audit", "scan", "--audit-bin"])
        .arg(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    let output = command::bounded_output(&mut command, Duration::from_secs(30), 64 * 1024)
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let arguments: Vec<String> = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
    assert_eq!(
        arguments,
        vec![
            "audit".into(),
            "--file".into(),
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("Cargo.lock")
                .canonicalize()
                .unwrap()
                .display()
                .to_string(),
            "--db".into(),
            database.display().to_string(),
            "--no-fetch".into(),
            "--no-yanked".into(),
        ]
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("RustSec advisory database before scan:"));
    assert!(stdout.contains("RustSec advisory database after scan:"));
}

#[tokio::test]
async fn cli_rejects_missing_dirty_attached_stale_and_wrong_origin_databases_before_scanning() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing-database");
    let dirty_root = root.path().join("dirty");
    fs::create_dir(&dirty_root).unwrap();
    let dirty = advisory_database(&dirty_root).await;
    fs::write(dirty.join("untracked"), b"dirty\n").unwrap();

    let attached_root = root.path().join("attached");
    fs::create_dir(&attached_root).unwrap();
    let attached = advisory_database(&attached_root).await;
    git(&attached, &["checkout", "-b", "fixture-attached"]).await;

    let stale_root = root.path().join("stale");
    fs::create_dir(&stale_root).unwrap();
    let stale = advisory_database(&stale_root).await;
    git_with_environment(
        &stale,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "stale fixture",
            "--date=2000-01-01T00:00:00Z",
        ],
        &[
            ("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z"),
        ],
    )
    .await;

    let wrong_origin_root = root.path().join("wrong-origin");
    fs::create_dir(&wrong_origin_root).unwrap();
    let wrong_origin = advisory_database(&wrong_origin_root).await;
    git(
        &wrong_origin,
        &[
            "config",
            "remote.origin.url",
            "https://example.invalid/not-rustsec.git",
        ],
    )
    .await;

    for (name, database, expected) in [
        ("missing", missing.as_path(), "advisory database is missing"),
        ("dirty", dirty.as_path(), "advisory database must be clean"),
        (
            "attached",
            attached.as_path(),
            "advisory database HEAD must be detached",
        ),
        (
            "stale",
            stale.as_path(),
            "advisory database is older than 90 days",
        ),
        (
            "wrong-origin",
            wrong_origin.as_path(),
            "advisory database local configuration is not an accepted fresh clone",
        ),
    ] {
        let marker = root.path().join(format!("{name}-capture"));
        let output = scan_cli(database, &marker, false, false, None).await;
        assert!(!output.status.success(), "{name} unexpectedly scanned");
        assert!(
            !marker.exists(),
            "{name} invoked the scanner before rejecting its database"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{name}: {stderr}");
    }
}

#[tokio::test]
async fn cli_rejects_changed_advisory_head_and_propagates_scanner_failure() {
    let root = tempfile::tempdir().unwrap();
    for (name, advance_head, fail, expected) in [
        (
            "changed-head",
            true,
            false,
            "advisory database changed during offline scan",
        ),
        ("scanner-failure", false, true, "cargo-audit scan failed"),
    ] {
        let case_root = root.path().join(name);
        fs::create_dir(&case_root).unwrap();
        let database = advisory_database(&case_root).await;
        let marker = root.path().join(format!("{name}-capture"));
        let output = scan_cli(&database, &marker, advance_head, fail, None).await;
        assert!(!output.status.success(), "{name} unexpectedly succeeded");
        assert!(
            marker.is_file(),
            "{name} did not invoke the scanner fixture"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{name}: {stderr}");
    }
}

#[tokio::test]
async fn cli_uses_owned_project_audit_configuration_despite_hostile_cargo_home() {
    let root = tempfile::tempdir().unwrap();
    let database = advisory_database(root.path()).await;
    let marker = root.path().join("audit-arguments.json");
    let environment_marker = root.path().join("audit-environment.json");
    let hostile_cargo_home = root.path().join("hostile-cargo-home");
    fs::create_dir(&hostile_cargo_home).unwrap();
    fs::write(
        hostile_cargo_home.join("audit.toml"),
        b"this is deliberately not valid TOML = [\n",
    )
    .unwrap();

    let mut command = command::Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("KURU_ADVISORY_DB", &database)
        .env("KURU_AUDIT_CAPTURE", &marker)
        .env("KURU_AUDIT_CAPTURE_ENV", &environment_marker)
        .env("CARGO_HOME", &hostile_cargo_home)
        .args(["audit", "scan", "--audit-bin"])
        .arg(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    let output = command::bounded_output(&mut command, Duration::from_secs(30), 64 * 1024)
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let environment: serde_json::Value =
        serde_json::from_slice(&fs::read(environment_marker).unwrap()).unwrap();
    assert_eq!(
        Path::new(environment["current_directory"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap(),
    );
    assert_ne!(
        environment["cargo_home"],
        hostile_cargo_home.display().to_string(),
        "scanner inherited the hostile Cargo Home configuration"
    );
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".cargo/audit.toml")
            .is_file(),
        "scanner must run with the owned project cargo-audit configuration"
    );
}

#[cfg(unix)]
fn fixture(arguments: &[&str]) -> command::Command {
    let mut command = command::Command::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    command.args(arguments);
    command
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_output_reaps_owned_group_after_stdout_or_stderr_overflow() {
    for stream in ["stdout", "stderr"] {
        let mut child = fixture(&["bounded-overflow", stream]);
        let error = command::bounded_output(&mut child, Duration::from_secs(5), 1024)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("tool output exceeds limit"),
            "{stream}: {error}"
        );
        assert!(
            error.contains("owned process group stopped and root reaped"),
            "{stream}: {error}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_output_reaps_clean_natural_exit_after_both_pipes_close() {
    let mut child = fixture(&["echo", "bounded", "exit"]);
    let output = command::bounded_output(&mut child, Duration::from_secs(5), 1024)
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.len() <= 1024);
    assert!(output.stderr.len() <= 1024);
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_output_polls_unreaped_root_after_pipes_close_before_exit() {
    let mut child = fixture(&["bounded-close-output-before-exit"]);
    let output = command::bounded_output(&mut child, Duration::from_secs(2), 1024)
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_output_cleans_silent_descendant_before_reaping_successful_root() {
    use kuru_platform::unix::observe_group_after_reap;

    let root = tempfile::tempdir().unwrap();
    let identity = root.path().join("root-group");
    let ready = root.path().join("silent-ready");
    let mut child = fixture(&[
        "bounded-silent-descendant-root",
        identity.to_str().expect("fixture identity path is UTF-8"),
        ready.to_str().expect("fixture readiness path is UTF-8"),
    ]);
    let output = command::bounded_output(&mut child, Duration::from_secs(2), 1024)
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(fs::read(&ready).unwrap(), b"ready");
    let marker = fs::read_to_string(identity).unwrap();
    let mut fields = marker.split_whitespace();
    let root_pid: i32 = fields.next().unwrap().parse().unwrap();
    let group: i32 = fields.next().unwrap().parse().unwrap();
    assert!(fields.next().is_none());
    assert_eq!(root_pid, group);
    // Read-only: the group is gone, or only another user's processes reuse it.
    let observed = observe_group_after_reap(group.unsigned_abs());
    assert!(observed.none_of_ours(), "{observed}");
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_output_times_out_and_reaps_descendant_holding_inherited_output() {
    use kuru_platform::unix::observe_group_after_reap;

    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("process-group");
    let mut child = fixture(&[
        "bounded-timeout-descendant",
        marker.to_str().expect("fixture marker path is UTF-8"),
    ]);
    let error = command::bounded_output(&mut child, Duration::from_millis(250), 64 * 1024)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("tool timed out"), "{error}");
    assert!(
        error.contains("owned process group stopped and root reaped"),
        "{error}"
    );
    let marker = fs::read_to_string(marker).unwrap();
    let mut identity = marker.split_whitespace();
    let pid: i32 = identity.next().unwrap().parse().unwrap();
    let group: i32 = identity.next().unwrap().parse().unwrap();
    assert!(identity.next().is_none());
    assert_eq!(
        pid, group,
        "the helper must create a fresh process group led by its root"
    );
    let observed = observe_group_after_reap(pid.unsigned_abs());
    assert!(
        observed.none_of_ours(),
        "fixture process group survived bounded cleanup: {observed}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_output_timeout_names_the_command_and_its_blocked_root() {
    let root = tempfile::tempdir().unwrap();
    let tag = root.path().join("timeout-tree");
    let tag = tag.to_str().expect("fixture tag path is UTF-8");
    let mut child = fixture(&["bounded-blocking-tree", "block", tag]);
    child.current_dir(root.path());
    let error = command::bounded_output(&mut child, Duration::from_millis(250), 64 * 1024)
        .await
        .unwrap_err()
        .to_string();
    for required in [
        "tool timed out",
        "owned process group stopped and root reaped",
        &format!("command={:?}", env!("CARGO_BIN_EXE_kuru-delivery-fixture")),
        &format!(r#"arguments=["bounded-blocking-tree", "block", "{tag}"]"#),
        &format!("directory=Some({:?})", root.path()),
        "root=running",
        &format!("bounded-blocking-tree block {tag}"),
    ] {
        assert!(error.contains(required), "missing {required}: {error}");
    }
    assert!(!error.contains("snapshot unavailable"), "{error}");
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_output_failure_snapshot_lists_the_live_grandchild_before_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let tag = root.path().join("overflow-tree");
    let tag = tag.to_str().expect("fixture tag path is UTF-8");
    let mut child = fixture(&["bounded-blocking-tree", "overflow", tag]);
    let error = command::bounded_output(&mut child, Duration::from_secs(30), 1024)
        .await
        .unwrap_err()
        .to_string();
    for required in [
        "tool output exceeds limit",
        "owned process group stopped and root reaped",
        "root=running",
        &format!("bounded-blocking-tree overflow {tag}"),
        // The grandchild was ready before the root overflowed its capture.
        &format!("bounded-ready-descendant {tag}"),
    ] {
        assert!(error.contains(required), "missing {required}: {error}");
    }
}

// A missing working directory fails the launch at once, so the failure text of
// each helper is checked without waiting for a 10-second deadline.
#[tokio::test]
async fn git_helpers_name_arguments_directory_and_elapsed_time_when_the_launch_fails() {
    async fn message(task: impl Future<Output = ()> + Send + 'static) -> String {
        let panic = tokio::spawn(task).await.unwrap_err().into_panic();
        panic
            .downcast_ref::<String>()
            .cloned()
            .expect("helper panics with a formatted message")
    }

    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("absent");
    let plain = message({
        let missing = missing.clone();
        async move { git(&missing, &["status", "--short"]).await }
    })
    .await;
    let with_environment = message({
        let missing = missing.clone();
        async move { git_with_environment(&missing, &["log"], &[("GIT_PAGER", "cat")]).await }
    })
    .await;
    for (text, arguments) in [
        (plain, r#"git ["status", "--short"]"#),
        (with_environment, r#"git ["log"]"#),
    ] {
        for required in [arguments, &format!("in {missing:?} failed after ")] {
            assert!(text.contains(required), "missing {required}: {text}");
        }
    }
}

#[tokio::test]
async fn fixture_head_advance_names_git_arguments_directory_and_elapsed_time_when_it_fails() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("absent");
    let mut child = command::Command::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    child
        .arg("--db")
        .arg(&missing)
        .env("KURU_AUDIT_CAPTURE", root.path().join("capture.json"))
        .env("KURU_AUDIT_ADVANCE_HEAD", "1");
    let output = command::bounded_output(&mut child, Duration::from_secs(30), 64 * 1024)
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    for required in [
        "controlled scanner git [",
        "controlled scanner changed advisory HEAD",
        // The fixture's `Error: Custom {..}` report escapes the quoted path.
        "absent",
        " failed after ",
    ] {
        assert!(stderr.contains(required), "missing {required}: {stderr}");
    }
}
