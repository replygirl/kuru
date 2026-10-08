#![cfg(unix)]

use kuru_platform::unix::{GroupObservation, observe_group_after_reap, own_uids, snapshot};
use rustix::process::{
    Pid, Signal, WaitId, WaitIdOptions, kill_process_group, test_kill_process_group, waitid,
};
use std::{
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    sync::{Mutex, MutexGuard},
};

// These fixtures require admitted snapshots, rather than concurrent spawn
// contention. Keep their setup isolated without spending observation deadlines.
static SNAPSHOT_FIXTURE: Mutex<()> = Mutex::new(());

fn snapshot_fixture() -> MutexGuard<'static, ()> {
    SNAPSHOT_FIXTURE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[test]
fn snapshot_lists_a_blocked_root_its_child_and_grandchild() {
    let _fixture = snapshot_fixture();
    // The subshell forks its sleeping child before printing readiness, so all
    // three processes exist once the line arrives.
    let mut root = Command::new("/bin/sh")
        .args([
            "-c",
            "(/bin/sleep 37 & echo ready; wait) & wait",
            "kuru-snapshot-root",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(root.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "ready\n");
    let id = root.id();
    let rows = snapshot::tree(id);
    let membership = snapshot::group_members_until_with(
        Path::new(snapshot::PS),
        id,
        std::time::Instant::now() + snapshot::SNAPSHOT_TIMEOUT,
    );
    let text = snapshot::describe(id);
    // The unreaped root still anchors its group for this owned cleanup.
    kill_process_group(Pid::from_raw(id as i32).unwrap(), Signal::KILL).unwrap();
    root.wait().unwrap();

    let rows = rows.unwrap();
    let membership = membership.unwrap();
    assert_eq!(
        membership.iter().map(|row| row.pid).collect::<Vec<_>>(),
        rows.iter().map(|row| row.pid).collect::<Vec<_>>(),
        "minimal membership disagreed with the blocked tree: {membership:?} {text}"
    );
    assert!(
        membership
            .iter()
            .all(|row| row.pgid == id && !row.state.is_empty())
    );
    assert!(rows.iter().any(|row| row.pid == id), "{text}");
    let child = rows
        .iter()
        .find(|row| row.ppid == id)
        .unwrap_or_else(|| panic!("no child of the root: {text}"));
    assert!(
        rows.iter().any(|row| row.ppid == child.pid),
        "no grandchild: {text}"
    );
    assert!(rows.iter().all(|row| row.pgid == id), "{text}");
    assert!(
        text.starts_with(&format!("process tree of {id} (")),
        "{text}"
    );
    assert!(text.contains(&format!("pid={id} ")), "{text}");
}

#[test]
fn snapshot_failure_is_reported_as_text() {
    let _fixture = snapshot_fixture();
    let text = snapshot::describe_with(Path::new("/nonexistent/kuru-ps"), std::process::id());
    assert!(text.starts_with("snapshot unavailable: spawn "), "{text}");
    let failing = snapshot::describe_with(Path::new("/usr/bin/false"), std::process::id());
    assert!(
        failing.starts_with("snapshot unavailable: ") && failing.contains("failed with"),
        "{failing}"
    );
}

/// Writes `script` to `path` and marks it executable without ever opening the
/// file in this process.
///
/// Creating the stand-in here (`std::fs::write`) would hold a write descriptor
/// for a moment, and the sibling tests in this binary fork children from other
/// threads. `fork` copies the whole descriptor table and `FD_CLOEXEC` closes the
/// copy only at the child's `exec`, so a child forked inside that moment keeps
/// the file open for writing until then, and Linux refuses to execute a file that
/// is open for writing (`ETXTBSY`, "Text file busy"). Letting a short-lived
/// shell hold the only write descriptor, and reaping it before the file is used,
/// leaves no descriptor here for a sibling to copy.
fn write_executable_from_child(path: &Path, script: &str) {
    let status = Command::new("/bin/sh")
        .args(["-c", r#"printf '%s' "$2" > "$1" && chmod 755 "$1""#])
        .arg("kuru-stand-in")
        .arg(path)
        .arg(script)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "stand-in creation exited with {status}");
}

#[test]
fn membership_selector_is_exact_under_an_inherited_legacy_environment() {
    let _fixture = snapshot_fixture();
    const CHILD: &str = "KURU_PS_SELECTOR_FIXTURE";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "membership_selector_is_exact_under_an_inherited_legacy_environment",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("COMMAND_MODE", "legacy")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child selector check failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    assert_eq!(std::env::var("COMMAND_MODE").unwrap(), "legacy");
    let root = tempfile::tempdir().unwrap();
    let program = root.path().join("selector-ps");
    #[cfg(target_os = "macos")]
    let selection = "test \"$COMMAND_MODE\" = unix2003 && test \"$#\" = 5 && test \"$1\" = -g && test \"$2\" = 20 && test \"$3\" = -x && test \"$4\" = -o && test \"$5\" = 'pid=,pgid=,stat='";
    #[cfg(not(target_os = "macos"))]
    let selection = "test \"$#\" = 3 && test \"$1\" = -A && test \"$2\" = -o && test \"$3\" = 'pid=,pgid=,stat='";
    write_executable_from_child(
        &program,
        &format!(
            "#!/bin/sh\n{selection} && test \"$LC_ALL\" = C || exit 23\nprintf '20 20 Z\\n21 20 S\\n22 99 R\\n'\n"
        ),
    );
    let members = snapshot::group_members_until_with(
        &program,
        20,
        std::time::Instant::now() + snapshot::SNAPSHOT_TIMEOUT,
    )
    .unwrap();
    assert_eq!(
        members.iter().map(|row| row.pid).collect::<Vec<_>>(),
        [20, 21]
    );
    assert!(members[0].state.starts_with('Z'));
}

#[test]
fn membership_rejects_success_status_with_stderr_while_diagnostics_keep_it() {
    let _fixture = snapshot_fixture();
    let root = tempfile::tempdir().unwrap();
    let program = root.path().join("failed-sysctl-ps");
    write_executable_from_child(
        &program,
        "#!/bin/sh\nprintf 'Failure calling sysctl\\n' >&2\nexit 0\n",
    );
    let error = snapshot::group_members_until_with(
        &program,
        20,
        std::time::Instant::now() + snapshot::SNAPSHOT_TIMEOUT,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("reported an inspection error: Failure calling sysctl"),
        "{error}"
    );
    assert!(snapshot::tree_with(&program, 20).unwrap().is_empty());
}

/// Descriptors of this process that refer to the file at `path`, found by device
/// and inode so symlinked temporary directories do not hide a match. Linux only:
/// `/proc/self/fd` resolves each entry to the open file, and Linux is the kernel
/// that refuses to execute a file open for writing. macOS and the BSDs execute
/// such a file, and their `/dev/fd` reports another device, so a descriptor
/// leaked there would be neither harmful nor reliably visible.
#[cfg(target_os = "linux")]
fn descriptors_referring_to(path: &Path) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let target = std::fs::metadata(path).unwrap();
    let mut found = Vec::new();
    // Entries that vanish or cannot be followed (the directory handle of this
    // very read, sockets, pipes) are not the file.
    for entry in std::fs::read_dir("/proc/self/fd").unwrap().flatten() {
        if let Ok(metadata) = std::fs::metadata(entry.path())
            && metadata.dev() == target.dev()
            && metadata.ino() == target.ino()
        {
            found.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    found
}

#[test]
fn snapshot_helper_is_bounded_when_ps_does_not_finish() {
    let _fixture = snapshot_fixture();
    let root = tempfile::tempdir().unwrap();
    // `exec` keeps the stand-in a single process, like `ps`, so stopping it
    // through its owned handle also closes both pipes.
    let stalled = root.path().join("stalled-ps");
    write_executable_from_child(&stalled, "#!/bin/sh\nexec /bin/sleep 30\n");
    // Nothing in this process may hold the stand-in open, or a concurrently
    // forked sibling could inherit a write descriptor that makes its exec fail.
    #[cfg(target_os = "linux")]
    assert_eq!(descriptors_referring_to(&stalled), Vec::<String>::new());
    let started = std::time::Instant::now();
    let text = snapshot::describe_with(&stalled, std::process::id());
    let elapsed = started.elapsed();
    assert!(
        text.starts_with("snapshot unavailable: ") && text.contains("did not finish within"),
        "{text}"
    );
    assert!(elapsed >= snapshot::SNAPSHOT_TIMEOUT, "{elapsed:?}");
    assert!(
        elapsed < snapshot::SNAPSHOT_TIMEOUT + std::time::Duration::from_secs(3),
        "{elapsed:?}"
    );
}

#[test]
fn absolute_snapshot_deadline_reaps_its_stalled_helper() {
    let _fixture = snapshot_fixture();
    let root = tempfile::tempdir().unwrap();
    let stalled = root.path().join("deadline-ps");
    let marker = root.path().join("helper-id");
    write_executable_from_child(
        &stalled,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nexec /bin/sleep 30\n",
            marker.display()
        ),
    );
    let start = std::time::Instant::now();
    let result = snapshot::group_members_until_with(
        &stalled,
        std::process::id(),
        start + std::time::Duration::from_millis(100),
    );
    let elapsed = start.elapsed();
    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error}");
    // The absolute deadline may expire after spawn but before the interpreter
    // writes its first marker, or during that write. Only a complete line names
    // the exact helper; partial decimal bytes must not select another PID.
    let helper = match std::fs::read_to_string(&marker) {
        Ok(text) => text
            .strip_suffix('\n')
            .and_then(|id| id.parse::<i32>().ok())
            .and_then(Pid::from_raw),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("read native helper evidence: {error}"),
    };
    let rows: Vec<_> = snapshot::processes()
        .unwrap()
        .into_iter()
        .filter(|row| {
            // Every native child fixture in this binary holds SNAPSHOT_FIXTURE.
            // A killed pre-marker helper loses its executable command as a
            // zombie, so also require no unreaped child of this exact parent.
            row.ppid == std::process::id()
                && (row.command.contains(stalled.to_str().unwrap()) || row.state.starts_with('Z'))
        })
        .collect();
    assert!(
        rows.is_empty(),
        "helper remained owned after listing: {rows:?}"
    );
    if let Some(helper) = helper {
        assert_eq!(
            waitid(
                WaitId::Pid(helper),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .unwrap_err(),
            rustix::io::Errno::CHILD,
            "the complete native helper identity was not reaped",
        );
    }
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
}

#[test]
fn expired_absolute_snapshot_admission_does_not_spawn() {
    let _fixture = snapshot_fixture();
    let error = snapshot::group_members_until_with(
        Path::new("/nonexistent/kuru-late-ps"),
        std::process::id(),
        std::time::Instant::now(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error}");
}

#[cfg(target_os = "linux")]
#[test]
fn descriptor_scan_sees_exactly_the_descriptors_held_here() {
    let _fixture = snapshot_fixture();
    // The scan in the bounded test is only evidence if it can find a descriptor.
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("held");
    write_executable_from_child(&file, "#!/bin/sh\n");
    assert_eq!(descriptors_referring_to(&file), Vec::<String>::new());
    let held = std::fs::File::open(&file).unwrap();
    let found = descriptors_referring_to(&file);
    drop(held);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(descriptors_referring_to(&file), Vec::<String>::new());
}

#[cfg(target_os = "linux")]
#[test]
fn a_write_descriptor_inherited_by_a_live_child_blocks_exec_of_the_file() {
    let _fixture = snapshot_fixture();
    // The mechanism behind the stand-in's creation rule, without the fork race:
    // a live child holds a write descriptor to the file (as a sibling's child does
    // between fork and exec), so the kernel refuses to execute it. Only the first
    // attempt is asserted: once the holder is gone the result depends on whatever
    // other thread forks next, which is exactly what the creation rule avoids.
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("busy");
    write_executable_from_child(&file, "#!/bin/sh\nexec /bin/sleep 30\n");
    let writer = std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap();
    // Standard streams are not close-on-exec, so the child keeps the descriptor.
    let mut holder = Command::new("/bin/sleep")
        .arg("37")
        .stdin(Stdio::null())
        .stdout(Stdio::from(writer))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let refused = Command::new(&file).spawn();
    holder.kill().unwrap();
    holder.wait().unwrap();
    let error = refused.expect_err("exec of a file open for writing must be refused");
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::ExecutableFileBusy,
        "{error}"
    );
}

#[test]
fn still_listed_reports_recorded_processes_until_they_are_gone() {
    let _fixture = snapshot_fixture();
    // No shell: the ID and command are final the moment `spawn` returns.
    let mut root = Command::new("/bin/sleep")
        .arg("37")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let id = root.id();
    let recorded = snapshot::tree(id).unwrap();
    assert!(recorded.iter().any(|row| row.pid == id));
    let live = snapshot::describe_still_listed(&recorded);
    let live_rows = snapshot::still_listed(&recorded).unwrap();
    kill_process_group(Pid::from_raw(id as i32).unwrap(), Signal::KILL).unwrap();
    root.wait().unwrap();
    let gone = snapshot::describe_still_listed(&recorded);
    let gone_rows = snapshot::still_listed(&recorded).unwrap();
    assert!(live_rows.iter().any(|row| row.pid == id), "{live_rows:?}");
    assert!(gone_rows.is_empty(), "{gone_rows:?}");
    assert!(snapshot::still_listed_with(Path::new("/nonexistent/kuru-ps"), &recorded).is_err());

    assert!(live.contains("recorded processes remain listed"), "{live}");
    assert!(live.contains(&format!("pid={id} ")), "{live}");
    assert!(
        gone.starts_with(&format!("none of {} recorded processes", recorded.len())),
        "{gone}"
    );
    let failed = snapshot::describe_still_listed_with(Path::new("/nonexistent/kuru-ps"), &recorded);
    assert!(failed.starts_with("snapshot unavailable: "), "{failed}");
}

#[test]
fn an_unreaped_child_is_listed_under_its_parent_but_not_as_its_recorded_row() {
    let _fixture = snapshot_fixture();
    // A killed child no longer runs its command (macOS lists `<defunct>`), so
    // a check that an owned child was reaped must key on its ID and this
    // parent, not on the row recorded while it ran.
    let mut child = Command::new("/bin/sleep")
        .arg("37")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let id = child.id();
    let parent = std::process::id();
    let recorded: Vec<_> = snapshot::tree(id)
        .unwrap()
        .into_iter()
        .filter(|row| row.pid == id)
        .collect();
    child.kill().unwrap();
    // Wait for the exit without consuming it, leaving an unreaped zombie.
    waitid(
        WaitId::Pid(Pid::from_raw(id as i32).unwrap()),
        WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
    )
    .unwrap();
    let ours = |rows: Vec<snapshot::ProcessRow>| -> Vec<_> {
        rows.into_iter()
            .filter(|row| row.pid == id && row.ppid == parent)
            .collect()
    };
    let unreaped = ours(snapshot::processes().unwrap());
    let membership = snapshot::group_members_until_with(
        Path::new(snapshot::PS),
        id,
        std::time::Instant::now() + snapshot::SNAPSHOT_TIMEOUT,
    );
    let by_recorded_row = snapshot::still_listed(&recorded).unwrap();
    child.wait().unwrap();
    let membership = membership.unwrap();
    assert_eq!(membership.len(), 1, "{membership:?}");
    assert_eq!(membership[0].pid, id);
    assert!(membership[0].state.starts_with('Z'), "{membership:?}");
    let reaped = ours(snapshot::processes().unwrap());
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(unreaped.len(), 1, "{unreaped:?}");
    assert!(unreaped[0].state.starts_with('Z'), "{unreaped:?}");
    assert!(by_recorded_row.is_empty(), "{by_recorded_row:?}");
    assert!(reaped.is_empty(), "{reaped:?}");
}

/// A process group led by another user's long-lived process, with no member
/// of ours: the state of a reaped group whose number another user took.
fn foreign_group() -> u32 {
    let own = own_uids();
    assert!(
        !own.contains(&0),
        "this check needs an unprivileged runner; uid 0 may signal every group"
    );
    let rows = snapshot::processes().unwrap();
    let ours = |row: &snapshot::ProcessRow| own.contains(&row.uid) || own.contains(&row.ruid);
    let mut refused = Vec::new();
    for leader in rows.iter().filter(|row| row.pid > 1 && row.pid == row.pgid) {
        if rows.iter().any(|row| row.pgid == leader.pgid && ours(row)) {
            continue;
        }
        // The pre-fix observation: signal zero is refused, which the old
        // post-reap queries reported as an error rather than as absence.
        match test_kill_process_group(Pid::from_raw(leader.pgid as i32).unwrap()) {
            Err(rustix::io::Errno::PERM) => return leader.pgid,
            result => refused.push(format!("{}:{result:?}", leader.pgid)),
        }
    }
    panic!("no foreign process group refused signal zero; candidates={refused:?}");
}

#[test]
fn foreign_group_is_classified_as_recycled_without_error() {
    let _fixture = snapshot_fixture();
    let group = foreign_group();
    let observed = observe_group_after_reap(group);
    let GroupObservation::Recycled(members) = &observed else {
        panic!("group {group} was not classified as recycled: {observed}");
    };
    assert!(observed.none_of_ours(), "{observed}");
    let own = own_uids();
    assert!(
        members
            .iter()
            .all(|row| row.pgid == group && !own.contains(&row.uid) && !own.contains(&row.ruid)),
        "{observed}"
    );
    // The new leader holding the group number is the evidence of recycling.
    assert!(members.iter().any(|row| row.pid == group), "{observed}");
    assert!(
        observed.to_string().contains("recycled by another user"),
        "{observed}"
    );
}

#[test]
fn live_group_of_ours_is_classified_as_a_survivor_with_its_listing() {
    let _fixture = snapshot_fixture();
    let mut root = Command::new("/bin/sleep")
        .arg("41")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let id = root.id();
    let observed = observe_group_after_reap(id);
    // The unreaped root still anchors its group for this owned cleanup.
    kill_process_group(Pid::from_raw(id as i32).unwrap(), Signal::KILL).unwrap();
    root.wait().unwrap();
    let after_reap = observe_group_after_reap(id);

    let GroupObservation::Survivors(Ok(members)) = &observed else {
        panic!("live group was not a survivor: {observed}");
    };
    assert!(!observed.none_of_ours());
    assert!(
        members
            .iter()
            .any(|row| row.pid == id && own_uids().contains(&row.uid)),
        "{observed}"
    );
    assert!(
        observed.to_string().contains(&format!("pid={id} ")),
        "{observed}"
    );
    // The group is empty now unless another user's process already took it.
    assert!(after_reap.none_of_ours(), "{after_reap}");
}
