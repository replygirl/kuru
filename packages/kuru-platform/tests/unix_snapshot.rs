#![cfg(unix)]

use kuru_platform::unix::snapshot;
use rustix::process::{Pid, Signal, kill_process_group};
use std::{
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
};

#[test]
fn snapshot_lists_a_blocked_root_its_child_and_grandchild() {
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
    let text = snapshot::describe(id);
    // The unreaped root still anchors its group for this owned cleanup.
    kill_process_group(Pid::from_raw(id as i32).unwrap(), Signal::KILL).unwrap();
    root.wait().unwrap();

    let rows = rows.unwrap();
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

#[cfg(target_os = "linux")]
#[test]
fn descriptor_scan_sees_exactly_the_descriptors_held_here() {
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
    kill_process_group(Pid::from_raw(id as i32).unwrap(), Signal::KILL).unwrap();
    root.wait().unwrap();
    let gone = snapshot::describe_still_listed(&recorded);

    assert!(live.contains("recorded processes remain listed"), "{live}");
    assert!(live.contains(&format!("pid={id} ")), "{live}");
    assert!(
        gone.starts_with(&format!("none of {} recorded processes", recorded.len())),
        "{gone}"
    );
    let failed = snapshot::describe_still_listed_with(Path::new("/nonexistent/kuru-ps"), &recorded);
    assert!(failed.starts_with("snapshot unavailable: "), "{failed}");
}
