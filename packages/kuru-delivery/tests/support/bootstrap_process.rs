//! Retained Unix ownership for the real compiler-free bootstrap fixtures.

use std::{
    fs, io,
    path::Path,
    process::{ExitStatus, Output},
    sync::Arc,
    time::Duration,
};

use rustix::process::{
    Pid, Signal, WaitId, WaitIdOptions, kill_process_group, test_kill_process_group, waitid,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, ChildStderr, ChildStdout},
    time::{Instant, MissedTickBehavior},
};

const CAPTURE_LIMIT: usize = 4 * 1024 * 1024;
const EXCERPT_LIMIT: usize = 64 * 1024;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

struct Capture<R> {
    reader: Option<R>,
    bytes: Vec<u8>,
    total: u64,
    eof: bool,
    error: Option<String>,
}

impl<R: AsyncRead + Unpin> Capture<R> {
    fn new(reader: Option<R>) -> Self {
        Self {
            eof: reader.is_none(),
            reader,
            bytes: Vec::new(),
            total: 0,
            error: None,
        }
    }

    fn active(&self) -> bool {
        !self.eof && self.error.is_none()
    }

    async fn read(&mut self) {
        let mut chunk = [0; 8192];
        match self.reader.as_mut().unwrap().read(&mut chunk).await {
            Ok(0) => {
                self.eof = true;
                self.reader = None;
            }
            Ok(length) => {
                self.total = self.total.saturating_add(length as u64);
                let retained = length.min(CAPTURE_LIMIT.saturating_sub(self.bytes.len()));
                self.bytes.extend_from_slice(&chunk[..retained]);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn diagnostic(&self, name: &str) -> String {
        let mut excerpt = String::new();
        for byte in &self.bytes {
            let escaped = std::ascii::escape_default(*byte);
            if excerpt.len() + escaped.len() > EXCERPT_LIMIT {
                break;
            }
            excerpt.extend(escaped.map(char::from));
        }
        format!(
            "{name}_bytes={} {name}_eof={} {name}_error={:?} {name}_prefix=\"{excerpt}\"",
            self.total, self.eof, self.error,
        )
    }
}

#[derive(Clone, Copy)]
enum Root {
    Running,
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

impl Root {
    fn diagnostic(self) -> String {
        match self {
            Self::Running => "root=running".to_owned(),
            Self::Exited { code, signal } => format!(
                "root=exited root_exit={} root_signal={signal:?}",
                code.map_or_else(|| "none".to_owned(), |code| code.to_string()),
            ),
        }
    }
}

struct Owner {
    child: Option<Child>,
    pid: Pid,
    group_owned: bool,
    root_reaped: bool,
    cleanup_observed: bool,
    stdout: Capture<ChildStdout>,
    stderr: Capture<ChildStderr>,
    retained: Option<Arc<tempfile::TempDir>>,
}

impl Owner {
    fn new(mut child: Child, retained: Arc<tempfile::TempDir>) -> Self {
        let pid = Pid::from_raw(child.id().unwrap() as i32).unwrap();
        Self {
            stdout: Capture::new(child.stdout.take()),
            stderr: Capture::new(child.stderr.take()),
            child: Some(child),
            pid,
            group_owned: true,
            root_reaped: false,
            cleanup_observed: false,
            retained: Some(retained),
        }
    }

    fn observe(&mut self) -> io::Result<Root> {
        match waitid(
            WaitId::Pid(self.pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(None) => Ok(Root::Running),
            Ok(Some(status)) => Ok(Root::Exited {
                code: status.exit_status(),
                signal: status.terminating_signal(),
            }),
            Err(error) => {
                // A failed ownership query is never authority for a numeric
                // signal. In particular, ECHILD can mean somebody else reaped.
                self.group_owned = false;
                Err(error.into())
            }
        }
    }

    async fn capture_until(&mut self, deadline: Instant) -> Result<Root, String> {
        let mut observation = tokio::time::interval(Duration::from_millis(10));
        observation.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            let root = self
                .observe()
                .map_err(|error| format!("root=query-error: {error}"))?;
            if self.stdout.error.is_some() || self.stderr.error.is_some() {
                return Err(format!("capture failed; {}", root.diagnostic()));
            }
            if matches!(root, Root::Exited { .. }) && self.stdout.eof && self.stderr.eof {
                return Ok(root);
            }
            tokio::select! {
                biased;
                () = tokio::time::sleep_until(deadline) => {
                    let root = self.observe().map_err(|error| format!("root=query-error: {error}"))?;
                    return Err(format!("bootstrap timed out; {}", root.diagnostic()));
                }
                () = self.stdout.read(), if self.stdout.active() => {}
                () = self.stderr.read(), if self.stderr.active() => {}
                _ = observation.tick() => {}
            }
        }
    }

    async fn cleanup(&mut self, terminate: bool) -> (Option<ExitStatus>, String) {
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        let mut errors = Vec::new();
        if self.group_owned {
            if let Err(error) = self.observe() {
                errors.push(format!("ownership query: {error}"));
            }
        } else {
            errors.push("root ownership was lost; numeric signalling disarmed".to_owned());
        }
        if self.group_owned {
            if terminate
                && let Err(error) = kill_process_group(self.pid, Signal::KILL)
                && error != rustix::io::Errno::SRCH
            {
                errors.push(format!("owned group termination: {error}"));
            }
            // Natural completion must not hide leaked descendants by killing
            // them. Neither path may signal after consuming this root's anchor.
            self.group_owned = false;
        } else {
            return (None, format!("cleanup=unobserved errors={errors:?}"));
        }

        let mut status = None;
        let mut wait_failed = false;
        loop {
            if self.root_reaped && self.stdout.eof && self.stderr.eof {
                break;
            }
            if (self.root_reaped || wait_failed) && !self.stdout.active() && !self.stderr.active() {
                break;
            }
            tokio::select! {
                biased;
                () = tokio::time::sleep_until(deadline) => {
                    errors.push("five-second cleanup deadline elapsed".to_owned());
                    break;
                }
                result = self.child.as_mut().unwrap().wait(), if !self.root_reaped && !wait_failed => {
                    match result {
                        Ok(exit) => {
                            self.root_reaped = true;
                            status = Some(exit);
                        }
                        Err(error) => {
                            wait_failed = true;
                            errors.push(format!("root reap: {error}"));
                        }
                    }
                }
                () = self.stdout.read(), if self.stdout.active() => {}
                () = self.stderr.read(), if self.stderr.active() => {}
            }
        }
        if let Some(error) = &self.stdout.error {
            errors.push(format!("stdout: {error}"));
        }
        if let Some(error) = &self.stderr.error {
            errors.push(format!("stderr: {error}"));
        }
        if !terminate && self.root_reaped {
            // This is only an existence query. A recycled group can cause a
            // conservative rejection, never authority to signal it. In
            // particular, EPERM is not absence on either supported Unix OS.
            match test_kill_process_group(self.pid) {
                Err(rustix::io::Errno::SRCH) => {}
                Ok(()) => errors
                    .push("bootstrap left a surviving process group after natural exit".to_owned()),
                Err(error) => errors.push(format!("post-reap process group query: {error}")),
            }
        }
        self.cleanup_observed =
            errors.is_empty() && self.root_reaped && self.stdout.eof && self.stderr.eof;
        let diagnostic = format!(
            "cleanup={} root_reaped={} cleanup_stdout_eof={} cleanup_stderr_eof={} errors={errors:?}",
            if self.cleanup_observed {
                "observed"
            } else {
                "unobserved"
            },
            self.root_reaped,
            self.stdout.eof,
            self.stderr.eof,
        );
        (status, diagnostic)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if !self.cleanup_observed {
            if self.group_owned && self.observe().is_ok() {
                let _ = kill_process_group(self.pid, Signal::KILL);
                self.group_owned = false;
            }
            // Cancellation or unproven cleanup cannot delete private fixture
            // data or turn a lost wait identity into Child's numeric drop-kill.
            // Retain these owners for the remainder of this failed test process.
            std::mem::forget(self.child.take());
            std::mem::forget(self.stdout.reader.take());
            std::mem::forget(self.stderr.reader.take());
            std::mem::forget(self.retained.take());
        }
    }
}

fn stage_observations(parent: &Path) -> String {
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) => return format!("stage_query_error={error}"),
    };
    let mut observations = Vec::new();
    for entry in entries.take(8) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                observations.push(format!("entry_query_error={error}"));
                continue;
            }
        };
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(".kuru-install.")
        {
            continue;
        }
        match fs::symlink_metadata(entry.path()) {
            Ok(metadata) if metadata.is_dir() => {}
            result => {
                observations.push(format!("stage_type={result:?}"));
                continue;
            }
        }
        for name in [
            "SHA256SUMS",
            "archive.tar.gz",
            "archive.tar",
            "names",
            "types",
            "kuru",
            "stream",
        ] {
            let value = match fs::symlink_metadata(entry.path().join(name)) {
                Ok(metadata) => format!("type={:?},bytes={}", metadata.file_type(), metadata.len()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => "absent".to_owned(),
                Err(error) => format!("query-error={error}"),
            };
            observations.push(format!("{name}:{value}"));
        }
    }
    format!("known_stage_observations={observations:?}")
}

pub fn capture(
    child: Child,
    timeout: Duration,
    case: &str,
    retained: Arc<tempfile::TempDir>,
    stage_parent: &Path,
) -> impl std::future::Future<Output = Result<Output, String>> + Send + 'static {
    // Construct the guard before the future is first polled, so cancellation
    // of a queued control task cannot discard an unguarded live child.
    let mut owner = Owner::new(child, retained);
    let case = case.to_owned();
    let stage_parent = stage_parent.to_owned();
    async move {
        let started = Instant::now();
        let result = owner.capture_until(started + timeout).await;
        // Preserve the observations at failure, before cleanup changes EOF/state.
        let before = format!(
            "{} {}",
            owner.stdout.diagnostic("stdout"),
            owner.stderr.diagnostic("stderr"),
        );
        let capture_elapsed = started.elapsed();
        let stage = result
            .as_ref()
            .err()
            .map(|_| stage_observations(&stage_parent));
        let (status, cleanup) = owner.cleanup(result.is_err()).await;
        if result.is_err() || !owner.cleanup_observed {
            let path_label = if owner.cleanup_observed {
                "fixture_private_path"
            } else {
                "retained_private_path"
            };
            return Err(format!(
                "case={case} capture_elapsed_ms={} elapsed_ms={} original={} {before} {cleanup} {} {path_label}={:?}",
                capture_elapsed.as_millis(),
                started.elapsed().as_millis(),
                match result {
                    Err(error) => error,
                    Ok(root) => format!(
                        "capture completed but cleanup failed; {}",
                        root.diagnostic()
                    ),
                },
                stage.unwrap_or_else(|| stage_observations(&stage_parent)),
                owner.retained.as_ref().unwrap().path(),
            ));
        }
        Ok(Output {
            status: status.unwrap(),
            stdout: std::mem::take(&mut owner.stdout.bytes),
            stderr: std::mem::take(&mut owner.stderr.bytes),
        })
    }
}
