#![cfg(feature = "tooling")]

#[cfg(unix)]
use std::time::Duration;
use std::{
    fs,
    path::{Path, PathBuf},
};

use kuru_delivery::command;

const ORIGIN: &str = "https://github.com/RustSec/advisory-db.git";
const FETCH_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

#[path = "support/fixture_git.rs"]
mod fixture_git;
#[path = "support/launch_budget.rs"]
mod launch_budget;
use fixture_git::{FixtureGit, Templates};

async fn git(directory: &Path, arguments: &[&str]) {
    FixtureGit::new().git(directory, arguments).await;
}

async fn git_with_environment(directory: &Path, arguments: &[&str], environment: &[(&str, &str)]) {
    let fixture = FixtureGit::new();
    fixture
        .run(
            fixture.command(directory),
            directory,
            arguments,
            environment,
        )
        .await;
}

async fn advisory_database_with(fixture: &FixtureGit, root: &Path) -> PathBuf {
    let database = root.join("advisory-db");
    fs::create_dir(&database).unwrap();
    fixture.git(&database, &["init"]).await;
    fs::write(database.join("README.md"), b"fixture\n").unwrap();
    fixture.git(&database, &["add", "README.md"]).await;
    fixture
        .git(
            &database,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        )
        .await;
    fixture
        .git(&database, &["remote", "add", "origin", ORIGIN])
        .await;
    fixture
        .git(&database, &["config", "remote.origin.fetch", FETCH_REFSPEC])
        .await;
    fixture
        .git(&database, &["checkout", "--detach", "HEAD"])
        .await;
    database
}

async fn stale_commit(fixture: &FixtureGit, database: &Path) {
    fixture
        .run(
            fixture.command(database),
            database,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
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
}

/// Every advisory fixture repository this binary uses, built by one builder:
/// a clean detached checkout, and copies of it made attached, stale or given
/// the wrong origin. Each is `<variant>/advisory-db` under `root`.
async fn build_fixtures(fixture: &FixtureGit, root: &Path) {
    fs::create_dir(root.join("clean")).unwrap();
    let clean = advisory_database_with(fixture, &root.join("clean")).await;
    for variant in ["attached", "stale", "wrong-origin"] {
        fs::create_dir(root.join(variant)).unwrap();
        fixture_git::copy_tree(&clean, &root.join(variant).join("advisory-db"));
    }
    fixture
        .git(
            &root.join("attached/advisory-db"),
            &["checkout", "-b", "fixture-attached"],
        )
        .await;
    stale_commit(fixture, &root.join("stale/advisory-db")).await;
    fixture
        .git(
            &root.join("wrong-origin/advisory-db"),
            &[
                "config",
                "remote.origin.url",
                "https://example.invalid/not-rustsec.git",
            ],
        )
        .await;
}

/// The fixture repositories, built once per test binary under Cargo's
/// target tmp directory.
async fn templates() -> &'static Templates {
    static TEMPLATES: tokio::sync::OnceCell<Templates> = tokio::sync::OnceCell::const_new();
    TEMPLATES
        .get_or_init(|| async {
            let root = tempfile::Builder::new()
                .prefix("advisory-templates-")
                .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
                .unwrap();
            let fixture = FixtureGit::new();
            build_fixtures(&fixture, root.path()).await;
            Templates::seal(root, &fixture)
        })
        .await
}

/// A private copy of the `variant` fixture repository at `root/advisory-db`.
async fn advisory_database(variant: &str, root: &Path) -> PathBuf {
    let database = root.join("advisory-db");
    templates()
        .await
        .copy(&format!("{variant}/advisory-db"), &database);
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
    // The CLI's own bounds (advisory Git 60 s, cargo-audit scan 180 s) are
    // private to the library; the job deadline exceeds both.
    command::bounded_output(&mut command, launch_budget::until_job_deadline(), 64 * 1024)
        .await
        .unwrap()
}

#[tokio::test]
async fn cli_uses_direct_audit_binary_with_root_lockfile_and_offline_flags() {
    let root = tempfile::tempdir().unwrap();
    let database = advisory_database("clean", root.path()).await;
    let marker = root.path().join("audit-arguments.json");
    let mut command = command::Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("KURU_ADVISORY_DB", &database)
        .env("KURU_AUDIT_CAPTURE", &marker)
        .args(["audit", "scan", "--audit-bin"])
        .arg(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    let output =
        command::bounded_output(&mut command, launch_budget::until_job_deadline(), 64 * 1024)
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
    let dirty = advisory_database("clean", &dirty_root).await;
    fs::write(dirty.join("untracked"), b"dirty\n").unwrap();

    let attached_root = root.path().join("attached");
    fs::create_dir(&attached_root).unwrap();
    let attached = advisory_database("attached", &attached_root).await;

    let stale_root = root.path().join("stale");
    fs::create_dir(&stale_root).unwrap();
    let stale = advisory_database("stale", &stale_root).await;

    let wrong_origin_root = root.path().join("wrong-origin");
    fs::create_dir(&wrong_origin_root).unwrap();
    let wrong_origin = advisory_database("wrong-origin", &wrong_origin_root).await;

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
        let database = advisory_database("clean", &case_root).await;
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
    let database = advisory_database("clean", root.path()).await;
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
    let output =
        command::bounded_output(&mut command, launch_budget::until_job_deadline(), 64 * 1024)
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

/// Gives `command` a stdin pipe and returns its write end. The fixture's
/// keep-alive peers (the root and every descendant inherit it) read it to EOF,
/// so they block until the owned-group cleanup under test stops them, or at
/// the latest until the caller drops the returned writer.
#[cfg(unix)]
fn held_until_released(command: &mut command::Command) -> std::io::PipeWriter {
    let (reader, writer) = std::io::pipe().unwrap();
    command.stdin(reader);
    writer
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
    let _release = held_until_released(&mut child);
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
    // Read-only: the group is gone, or another user's new leader took its number.
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
    let _release = held_until_released(&mut child);
    // The timeout is the stimulus: it must fire after the root has written
    // its marker, while the released-only descendant holds both pipes.
    let error =
        command::bounded_output(&mut child, launch_budget::CHILD_START_ALLOWANCE, 64 * 1024)
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
    let _release = held_until_released(&mut child);
    // The timeout is the stimulus: it must fire after the root has started,
    // so the snapshot names its command line while it blocks.
    let error =
        command::bounded_output(&mut child, launch_budget::CHILD_START_ALLOWANCE, 64 * 1024)
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
    let _release = held_until_released(&mut child);
    let error = command::bounded_output(&mut child, launch_budget::until_job_deadline(), 1024)
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

/// The keep-alive peers end on their release, not a timer: a peer whose
/// writer is already dropped reads EOF and exits on its own, and the wait is
/// for that exit. The timeout tests above show the peers block while held.
#[cfg(unix)]
#[tokio::test]
async fn bounded_fixture_keep_alive_ends_on_its_release() {
    let mut child = fixture(&["bounded-held-output"]);
    drop(held_until_released(&mut child));
    let output = command::bounded_output(&mut child, launch_budget::until_job_deadline(), 1024)
        .await
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"descendant retained inherited output\n");
}

// A missing working directory fails the launch at once, so the failure text of
// each helper is checked without waiting for its deadline.
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
        // The launch failed before Git wrote its Trace2 file; the field says so.
        for required in [
            arguments,
            &format!("in {missing:?} failed after "),
            "; trace2 tail (",
            "trace2-0.json): <unavailable: ",
        ] {
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
    let output =
        command::bounded_output(&mut child, launch_budget::until_job_deadline(), 64 * 1024)
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
        "; trace2 tail (",
    ] {
        assert!(stderr.contains(required), "missing {required}: {stderr}");
    }
}

#[tokio::test]
async fn fixture_git_sequence_traces_every_call_and_starts_no_child_process() {
    // Sealing the binary's one template build already required a Trace2
    // start and no child_start from every call.
    let templates = templates().await;
    // Six calls for the clean checkout, then one per copied variant.
    assert_eq!(templates.calls(), 9);
    for variant in ["clean", "attached", "stale", "wrong-origin"] {
        assert!(
            templates
                .path()
                .join(variant)
                .join("advisory-db/.git")
                .is_dir()
        );
    }
}

#[tokio::test]
async fn fixture_git_reads_only_command_line_and_repository_configuration() {
    let root = tempfile::tempdir().unwrap();
    let database = advisory_database("clean", root.path()).await;
    let listing = FixtureGit::new()
        .git(&database, &["config", "--list", "--show-origin"])
        .await
        .stdout;
    fixture_git::assert_fixture_origins(&listing);
    let listing = String::from_utf8(listing).unwrap();
    for required in [
        "command line:\tmaintenance.auto=false",
        "command line:\tcore.fsmonitor=false",
        "file:.git/config\tremote.origin.url=",
    ] {
        assert!(listing.contains(required), "missing {required}: {listing}");
    }
}

const HOSTILE_CHILD: &str = "KURU_TEST_HOSTILE_FIXTURE_GIT_CHILD";

/// Write a script that records its own invocation as `markers/<role>`.
fn marker_script(path: &Path, markers: &Path, role: &str) {
    let marker = markers.join(role).display().to_string().replace('\\', "/");
    fs::write(
        path,
        format!("#!/bin/sh\necho \"$0 $*\" > '{marker}'\nexit 1\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn markers(root: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(root.join("markers"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Runs only in the child process the hostile-environment test starts.
async fn hostile_child(root: &Path) {
    let fixture = FixtureGit::new();
    let fixtures = root.join("fixtures");
    fs::create_dir(&fixtures).unwrap();
    build_fixtures(&fixture, &fixtures).await;
    fixture.assert_no_children();
    let listing = fixture
        .git(
            &fixtures.join("clean/advisory-db"),
            &["config", "--list", "--show-origin"],
        )
        .await
        .stdout;
    fixture_git::assert_fixture_origins(&listing);
    assert_eq!(markers(root), Vec::<String>::new(), "hostile programs ran");

    // Control: Git outside the builder reads the hostile configuration and
    // runs its hook. Only hooks stay enabled, so no daemon or maintenance
    // child outlives the control.
    let control = root.join("control");
    fs::create_dir(&control).unwrap();
    for arguments in [
        &["init"][..],
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "control",
        ],
    ] {
        let mut command = command::rooted(&control, "git");
        command
            .args(["-c", "core.fsmonitor=false", "-c", "maintenance.auto=false"])
            .args(["-c", "gc.auto=0", "-c", "commit.gpgsign=false"])
            .args(arguments);
        command::bounded_output(&mut command, fixture_git::bound(), 64 * 1024)
            .await
            .unwrap();
    }
    assert!(
        markers(root).contains(&"pre-commit".to_owned()),
        "the hostile configuration did not reach unisolated Git: {:?}",
        markers(root)
    );
}

#[tokio::test]
async fn fixture_git_ignores_hostile_inherited_configuration_and_programs() {
    if let Some(root) = std::env::var_os(HOSTILE_CHILD) {
        hostile_child(Path::new(&root)).await;
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let markers = root.path().join("markers");
    let hooks = root.path().join("hostile-hooks");
    let programs = root.path().join("hostile-programs");
    for directory in [&markers, &hooks, &programs] {
        fs::create_dir(directory).unwrap();
    }
    for hook in ["pre-commit", "post-commit"] {
        marker_script(&hooks.join(hook), &markers, hook);
    }
    for role in ["credential", "gpg", "askpass"] {
        marker_script(&programs.join(role), &markers, role);
    }
    let slash = |path: &Path| path.display().to_string().replace('\\', "/");
    let config = root.path().join("hostile.gitconfig");
    fs::write(
        &config,
        format!(
            "[core]\n\thooksPath = {}\n\tfsmonitor = true\n\
             [credential]\n\thelper = \"!{}\"\n\
             [maintenance]\n\tauto = true\n\
             [gc]\n\tauto = 1\n\
             [commit]\n\tgpgsign = true\n\
             [tag]\n\tgpgsign = true\n\
             [gpg]\n\tprogram = {}\n",
            slash(&hooks),
            slash(&programs.join("credential")),
            slash(&programs.join("gpg")),
        ),
    )
    .unwrap();
    // The hostile environment exists only in the child test process.
    let mut child = command::Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "fixture_git_ignores_hostile_inherited_configuration_and_programs",
            "--nocapture",
        ])
        .env(HOSTILE_CHILD, root.path())
        .env_remove("GIT_CONFIG_NOSYSTEM")
        .env("GIT_CONFIG_SYSTEM", &config)
        .env("GIT_CONFIG_GLOBAL", &config)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_0", &hooks)
        .env("GIT_ASKPASS", programs.join("askpass"))
        .env("SSH_ASKPASS", programs.join("askpass"))
        .env("GIT_TERMINAL_PROMPT", "1");
    // The re-executed test binary has no product budget, as for the
    // foreign-repository child in support/repository_environment.rs, so it
    // waits until the job deadline; each fixture Git call inside it keeps
    // its own bound and Trace2 tail.
    let output =
        command::bounded_output(&mut child, launch_budget::until_job_deadline(), 256 * 1024)
            .await
            .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(text.contains("1 passed"), "{text}");
}
