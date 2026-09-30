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
