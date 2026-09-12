#![cfg(unix)]

use kuru_platform::unix::{GroupPresence, OwnedProcessGroup, Reap, RootState, Termination};
use std::{
    fs,
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const LIMIT: Duration = Duration::from_secs(5);
const STDERR_LIMIT: usize = 8 * 1024;

struct StderrSummary {
    prefix: Vec<u8>,
    total: usize,
}

struct StderrCapture {
    receiver: Option<mpsc::Receiver<io::Result<StderrSummary>>>,
    worker: Option<JoinHandle<()>>,
    completed: Option<io::Result<StderrSummary>>,
}

impl StderrCapture {
    fn spawn(stderr: std::process::ChildStderr) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let result = drain_stderr(stderr);
            let _ = sender.send(result);
        });
        Self {
            receiver: Some(receiver),
            worker: Some(worker),
            completed: None,
        }
    }

    fn unavailable(error: io::Error) -> Self {
        Self {
            receiver: None,
            worker: None,
            completed: Some(Err(error)),
        }
    }

    fn finish(&mut self, timeout: Duration) -> (bool, String) {
        if self.completed.is_none() {
            let received = self
                .receiver
                .as_ref()
                .expect("pending stderr capture retains a receiver")
                .recv_timeout(timeout);
            match received {
                Ok(result) => {
                    self.completed = Some(result);
                    if let Some(worker) = self.worker.take()
                        && worker.join().is_err()
                    {
                        return (true, "stderr worker panicked".to_owned());
                    }
                    self.receiver = None;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return (false, "stderr reader remains pending".to_owned());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.completed = Some(Err(io::Error::other("stderr worker disconnected")));
                    self.receiver = None;
                    if let Some(worker) = self.worker.take() {
                        let _ = worker.join();
                    }
                }
            }
        }
        match self
            .completed
            .as_ref()
            .expect("stderr completion was recorded")
        {
            Ok(summary) if summary.total <= STDERR_LIMIT => (
                true,
                format!("stderr={}", String::from_utf8_lossy(&summary.prefix)),
            ),
            Ok(summary) => (
                true,
                format!(
                    "stderr exceeded {STDERR_LIMIT} bytes ({}), prefix={}",
                    summary.total,
                    String::from_utf8_lossy(&summary.prefix)
                ),
            ),
            Err(error) => (true, format!("stderr_read={error}")),
        }
    }
}

fn drain_stderr(stderr: std::process::ChildStderr) -> io::Result<StderrSummary> {
    let mut reader = BufReader::new(stderr);
    let mut prefix = Vec::new();
    let mut total = 0usize;
    let mut chunk = [0; 4096];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Ok(StderrSummary { prefix, total });
        }
        total = total.saturating_add(count);
        let retained = count.min(STDERR_LIMIT.saturating_sub(prefix.len()));
        prefix.extend_from_slice(&chunk[..retained]);
    }
}

struct LiveFixture {
    owner: OwnedProcessGroup,
    // The child uses this as its current directory. It must outlive either a
    // normal cleanup or a real background continuation.
    root: tempfile::TempDir,
    stderr: StderrCapture,
}

struct NativeFixture {
    live: Option<LiveFixture>,
    completed: bool,
}

#[derive(Debug)]
struct Cleanup {
    confirmed: bool,
    status: Option<std::process::ExitStatus>,
    detail: String,
}

impl NativeFixture {
    fn shell(script: &str, arguments: &[&Path]) -> io::Result<(Self, PathBuf)> {
        let root = tempfile::tempdir()?;
        let ready = root.path().join("root-ready");
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(script)
            .arg("kuru-unix-process-fixture")
            .args(arguments)
            .arg(&ready)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .current_dir(root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut owner = OwnedProcessGroup::spawn(command)?;
        let stderr = match owner.take_stderr() {
            Ok(stderr) => StderrCapture::spawn(stderr),
            Err(error) => StderrCapture::unavailable(error),
        };
        Ok((
            Self {
                live: Some(LiveFixture {
                    owner,
                    root,
                    stderr,
                }),
                completed: false,
            },
            ready,
        ))
    }

    fn owner(&mut self) -> &mut OwnedProcessGroup {
        &mut self
            .live
            .as_mut()
            .expect("fixture owner is present until confirmed cleanup")
            .owner
    }

    fn root(&self) -> &Path {
        self.live
            .as_ref()
            .expect("fixture root is present until confirmed cleanup")
            .root
            .path()
    }

    fn cleanup(&mut self) -> Cleanup {
        if self.completed {
            return Cleanup {
                confirmed: true,
                status: None,
                detail: "cleanup already confirmed".to_owned(),
            };
        }
        let live = self
            .live
            .as_mut()
            .expect("unconfirmed fixture retains its live continuation");
        let deadline = Instant::now() + LIMIT;
        let transition = loop {
            match live.owner.terminate_before_reap() {
                Termination::Signalled(report) => break Ok(format!("termination={report:?}")),
                // A prior cleanup can have consumed authority while awaiting
                // root exit. It must still be allowed to reap and prove group
                // absence rather than treating the phase guard as completion.
                Termination::InvalidPhase => break Ok("transition=already-consumed".to_owned()),
                Termination::Interrupted if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                result => break Err(format!("termination={result:?}")),
            }
        };
        let reap = if transition.is_ok() {
            loop {
                match live.owner.reap_if_exited() {
                    Reap::Reaped(status) => break Ok(status),
                    Reap::NotExited | Reap::Interrupted if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    result => break Err(format!("reap={result:?}")),
                }
            }
        } else {
            Err("reap unavailable after lost ownership".to_owned())
        };
        let absence = if reap.is_ok() {
            loop {
                match live.owner.presence_after_reap() {
                    GroupPresence::Absent => break Ok(()),
                    GroupPresence::Present | GroupPresence::PermissionDenied
                        if Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    result => break Err(format!("group_presence={result:?}")),
                }
            }
        } else {
            Err("absence unavailable before exact root reap".to_owned())
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        let (stderr_complete, stderr) = live
            .stderr
            .finish(remaining.min(Duration::from_millis(250)));
        let status = reap.as_ref().ok().copied();
        let confirmed = transition.is_ok() && reap.is_ok() && absence.is_ok() && stderr_complete;
        let detail = format!(
            "{transition:?}; {reap:?}; {absence:?}; {stderr}; fixture_root={}",
            live.root.path().display()
        );
        if confirmed {
            drop(self.live.take());
            self.completed = true;
        }
        Cleanup {
            confirmed,
            status,
            detail,
        }
    }
}

impl Drop for NativeFixture {
    fn drop(&mut self) {
        if self.completed || self.live.is_none() {
            return;
        }
        // Keep the actual owner, child pipes, and private root alive in a
        // continuation that may poll until it can reap. This is intentionally
        // not a cleanup-success claim and does not discard signal authority.
        let live = self
            .live
            .take()
            .expect("unconfirmed fixture has live state");
        thread::spawn(move || {
            let mut continuation = NativeFixture {
                live: Some(live),
                completed: false,
            };
            loop {
                let cleanup = continuation.cleanup();
                if cleanup.confirmed {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
    }
}

fn ready(path: &Path) -> Result<(), String> {
    let deadline = Instant::now() + LIMIT;
    loop {
        if path
            .try_exists()
            .map_err(|error| format!("fixture readiness query: {error}"))?
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("fixture readiness deadline elapsed".to_owned());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn exited(owner: &mut OwnedProcessGroup) -> Result<(), String> {
    let deadline = Instant::now() + LIMIT;
    loop {
        match owner.root_state() {
            RootState::Exited => return Ok(()),
            RootState::Running => {}
            state => return Err(format!("unexpected owned-root state: {state:?}")),
        }
        if Instant::now() >= deadline {
            return Err("owned root did not exit".to_owned());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn root_exit_with_same_group_descendant_is_signalled_before_exact_reap() {
    let script = r#"
        (
            printf descendant > descendant-started
            printf ready > descendant-ready
            exec sleep 30
        ) &
        printf root > "$1"
        exit 37
    "#;
    let (mut fixture, root_ready) = NativeFixture::shell(script, &[]).unwrap();
    let descendant_marker = fixture.root().join("descendant-started");
    let descendant_ready = fixture.root().join("descendant-ready");
    let observation = (|| -> Result<(), String> {
        ready(&root_ready)?;
        ready(&descendant_ready)?;
        exited(fixture.owner())?;
        let bytes = fs::read(&descendant_marker)
            .map_err(|error| format!("descendant marker read: {error}"))?;
        if bytes != b"descendant" {
            return Err(format!("unexpected descendant marker: {bytes:?}"));
        }
        Ok(())
    })();
    let cleanup = fixture.cleanup();
    assert!(cleanup.confirmed, "{}", cleanup.detail);
    assert_eq!(cleanup.status.and_then(|status| status.code()), Some(37));
    observation.unwrap();
}

#[test]
fn consumed_transition_still_reaps_and_confirms_absence() {
    let script = r#"
        printf root > "$1"
        exec sleep 30
    "#;
    let (mut fixture, root_ready) = NativeFixture::shell(script, &[]).unwrap();
    let observation = (|| -> Result<(), String> {
        ready(&root_ready)?;
        if !matches!(fixture.owner().reap_if_exited(), Reap::InvalidPhase) {
            return Err("reap was accepted before the transition".to_owned());
        }
        if !matches!(
            fixture.owner().presence_after_reap(),
            GroupPresence::InvalidPhase
        ) {
            return Err("presence was accepted before root reap".to_owned());
        }
        if !matches!(
            fixture.owner().terminate_before_reap(),
            Termination::Signalled(_)
        ) {
            return Err("first transition was not accepted".to_owned());
        }
        if !matches!(
            fixture.owner().terminate_before_reap(),
            Termination::InvalidPhase
        ) {
            return Err("repeated transition was not rejected".to_owned());
        }
        Ok(())
    })();
    let cleanup = fixture.cleanup();
    assert!(cleanup.confirmed, "{}", cleanup.detail);
    assert!(cleanup.status.is_some(), "{}", cleanup.detail);
    observation.unwrap();
}
