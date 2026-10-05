//! Isolated, traced Git for advisory fixtures.
//!
//! The delivery library (for its advisory unit tests), `tests/advisory.rs`
//! and the scanner fixture binary include this file with `#[path]`; each
//! includer has the delivery crate's `command` module and the sibling
//! `launch_budget` module (`launch_budget.rs`) in its own scope.
//!
//! A fixture Git process reads no system, global, ProgramData or XDG
//! configuration, runs no hook, credential helper, signing program, fsmonitor
//! daemon or auto-maintenance, and so starts no child process. Every call
//! writes its own Trace2 event file, and a failure names the trace's tail.
#![allow(dead_code)] // Each includer uses a subset.

use super::command;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

/// The bound of one fixture Git process, which starts no child and has no
/// product budget of its own: it waits for its exit and EOF until the coverage
/// job's inner test deadline (`launch_budget.rs`). Each includer has the
/// sibling `launch_budget` module in its own scope.
pub fn bound() -> Duration {
    super::launch_budget::until_job_deadline()
}
const OUTPUT_LIMIT: usize = 64 * 1024;
const TRACE_TAIL_LINES: usize = 40;

/// Configuration every fixture Git call receives on its command line.
pub const FLAGS: [&str; 7] = [
    "maintenance.auto=false",
    "gc.auto=0",
    "core.fsmonitor=false",
    "credential.helper=",
    "commit.gpgsign=false",
    "tag.gpgsign=false",
    "core.autocrlf=false",
];

pub struct FixtureGit {
    root: tempfile::TempDir,
    calls: AtomicUsize,
}

impl FixtureGit {
    pub fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("kuru-fixture-git-")
            .tempdir()
            .unwrap();
        fs::write(root.path().join("global.gitconfig"), b"").unwrap();
        for directory in ["hooks", "home", "traces"] {
            fs::create_dir(root.path().join(directory)).unwrap();
        }
        Self {
            root,
            calls: AtomicUsize::new(0),
        }
    }

    /// A rooted Git command with the isolated environment. Callers may add
    /// deliberate settings before [`FixtureGit::run`] adds the fixed flags.
    pub fn command(&self, directory: &Path) -> command::Command {
        let mut command = command::rooted(directory, "git");
        self.isolate(&mut command);
        command
    }

    fn isolate(&self, command: &mut command::Command) {
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy();
            if key.starts_with("GIT_CONFIG") || key.starts_with("GIT_TRACE2") {
                command.env_remove(key.as_ref());
            }
        }
        for key in ["GIT_ASKPASS", "SSH_ASKPASS"] {
            command.env_remove(key);
        }
        let home = self.root.path().join("home");
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                self.root.path().join("global.gitconfig"),
            )
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never");
    }

    /// Add the fixed flags and this call's own Trace2 event file.
    pub fn trace(&self, command: &mut command::Command) -> PathBuf {
        command.arg("-c").arg(format!(
            "core.hooksPath={}",
            self.root.path().join("hooks").display()
        ));
        for flag in FLAGS {
            command.args(["-c", flag]);
        }
        let trace = self.trace_path(self.calls.fetch_add(1, Ordering::Relaxed));
        command.env("GIT_TRACE2_EVENT", &trace);
        trace
    }

    pub async fn git(&self, directory: &Path, arguments: &[&str]) -> Output {
        self.run(self.command(directory), directory, arguments, &[])
            .await
    }

    /// Run `command` (from [`FixtureGit::command`]) within [`bound`]. A
    /// failure panics with the arguments, directory, elapsed time, the
    /// bounded runner's diagnostic and the call's Trace2 tail.
    pub async fn run(
        &self,
        mut command: command::Command,
        directory: &Path,
        arguments: &[&str],
        environment: &[(&str, &str)],
    ) -> Output {
        let trace = self.trace(&mut command);
        command.args(arguments).envs(environment.iter().copied());
        let started = Instant::now();
        let output = command::bounded_output(&mut command, bound(), OUTPUT_LIMIT)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "git {arguments:?} in {directory:?} failed after {:?}: {error}; {}",
                    started.elapsed(),
                    trace_tail(&trace)
                )
            });
        assert!(
            output.status.success(),
            "git {arguments:?} in {directory:?}: {}; {}",
            String::from_utf8_lossy(&output.stderr),
            trace_tail(&trace)
        );
        assert_childless(&trace);
        output
    }

    /// Fixture Git calls made through this builder.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    /// Assert every call wrote a Trace2 `start` and no call started a child.
    pub fn assert_no_children(&self) {
        assert!(self.calls() > 0, "no fixture Git call was traced");
        for call in 0..self.calls() {
            assert_childless(&self.trace_path(call));
        }
    }

    fn trace_path(&self, call: usize) -> PathBuf {
        self.root
            .path()
            .join("traces")
            .join(format!("trace2-{call}.json"))
    }
}

/// Parse one call's Trace2 event file.
pub fn events(trace: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(trace)
        .unwrap_or_else(|error| panic!("read fixture Trace2 {trace:?}: {error}"))
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// A fixture Git call wrote its Trace2 `start` and started no child process.
pub fn assert_childless(trace: &Path) {
    let events = events(trace);
    assert!(
        events.iter().any(|event| event["event"] == "start"),
        "fixture Git wrote no Trace2 start to {trace:?}: {events:?}"
    );
    let children: Vec<_> = events
        .iter()
        .filter(|event| event["event"] == "child_start")
        .collect();
    assert!(
        children.is_empty(),
        "fixture Git ({trace:?}) started children: {children:?}"
    );
}

/// Fixture repositories built once per test binary by one builder, which
/// each test copies, so a test itself starts no fixture Git process.
///
/// A template is held in a static for its binary's lifetime and is never
/// dropped, so its directory outlives the process.
pub struct Templates {
    root: tempfile::TempDir,
    calls: usize,
}

impl Templates {
    /// Seal the repositories `fixture` built under `root`, after checking that
    /// every call wrote its Trace2 start and none started a child.
    pub fn seal(root: tempfile::TempDir, fixture: &FixtureGit) -> Self {
        fixture.assert_no_children();
        Self {
            root,
            calls: fixture.calls(),
        }
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    /// Fixture Git calls the one build made.
    pub fn calls(&self) -> usize {
        self.calls
    }

    /// Copy the template tree `relative` to the new directory `destination`.
    pub fn copy(&self, relative: &str, destination: &Path) {
        copy_tree(&self.root.path().join(relative), destination);
    }
}

/// Copy a directory of plain files and directories into a new directory.
/// Each copy is a separate repository; no `.git` state is shared.
pub fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir(destination)
        .unwrap_or_else(|error| panic!("create fixture copy {destination:?}: {error}"));
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)
                .unwrap_or_else(|error| panic!("copy fixture file {:?}: {error}", entry.path()));
        } else {
            panic!(
                "fixture template entry {:?} is not a file or directory",
                entry.path()
            );
        }
    }
}

/// The `trace2 tail (<path>): ...` field of a fixture Git failure.
pub fn trace_tail(trace: &Path) -> String {
    let tail = match fs::read_to_string(trace) {
        Err(error) => format!("<unavailable: {error}>"),
        Ok(text) if text.trim().is_empty() => "<empty>".into(),
        Ok(text) => {
            let lines: Vec<_> = text.lines().collect();
            lines[lines.len().saturating_sub(TRACE_TAIL_LINES)..].join("\n")
        }
    };
    format!("trace2 tail ({}): {tail}", trace.display())
}

/// Assert `git config --list --show-origin` output names only the command
/// line and the repository's own `.git/config`, so no system, ProgramData,
/// global or XDG configuration reached a fixture Git process.
pub fn assert_fixture_origins(listing: &[u8]) {
    let listing = String::from_utf8_lossy(listing);
    for line in listing.lines() {
        let origin = line.split('\t').next().unwrap_or_default();
        assert!(
            origin == "command line:" || origin == "file:.git/config",
            "fixture Git read configuration from {origin:?}: {line}\n{listing}"
        );
    }
}
