//! The two explicit native source-update commands, with retained Unix ownership.

use anyhow::{Context, Result, ensure};
use kuru_platform::unix::{
    GroupPresence, OwnedProcessGroup, PreReap, Reap, RootState, StdioPlan, StdioSlot, Termination,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::oneshot,
    time::Instant,
};

const BUILD_TIMEOUT: Duration = Duration::from_secs(1800);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);
const LOG_LIMIT: usize = 64 * 1024;

#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("source update interrupted before publication")
    }
}
impl std::error::Error for Cancelled {}

struct Capture<R> {
    reader: R,
    bytes: Vec<u8>,
    eof: bool,
    omitted: bool,
    failed: bool,
}
impl<R: AsyncRead + Unpin> Capture<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            bytes: Vec::new(),
            eof: false,
            omitted: false,
            failed: false,
        }
    }
    async fn read(&mut self) -> std::io::Result<()> {
        let mut buffer = [0; 8192];
        let length = self.reader.read(&mut buffer).await?;
        self.eof = length == 0;
        let retained = length.min(LOG_LIMIT.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&buffer[..retained]);
        self.omitted |= retained < length;
        Ok(())
    }
}

fn cancelled(cancel: &mut oneshot::Receiver<()>) -> bool {
    !matches!(cancel.try_recv(), Err(oneshot::error::TryRecvError::Empty))
}

/// Build only; returned bytes are never executed or installed by this helper.
/// The caller retains the same cancellation receiver through both commands and
/// awaits this future after requesting cancellation, before releasing authority.
pub async fn build(source: &Path, mut cancel: oneshot::Receiver<()>) -> Result<PathBuf> {
    let source = source
        .canonicalize()
        .context("source checkout does not exist")?;
    let root = kuru_platform::fs::Directory::open(
        &source,
        kuru_platform::fs::Privacy::Inherited,
        kuru_platform::fs::NameRetention::Movable,
    )?;
    let host = crate::archive::host_target()?;
    if let Some(target) = std::env::var_os("CARGO_BUILD_TARGET") {
        ensure!(
            target == "host" || target == host,
            "source update requires the native target {host}"
        );
    }
    for arguments in [
        vec!["install", "rust"],
        vec![
            "run",
            "//apps/kuru-tui:build:release",
            "--",
            "--target",
            host,
        ],
    ] {
        if cancelled(&mut cancel) {
            return Err(Cancelled.into());
        }
        root.revalidate()?;
        let mut command = crate::command::rooted(&source, "mise");
        command
            .arg("-C")
            .arg(&source)
            .args(arguments)
            .env("MISE_NO_HOOKS", "1")
            .env("MISE_TASK_RUN_AUTO_INSTALL", "false")
            .env("KURU_MBX", "0")
            .env_remove("CARGO_BUILD_TARGET");
        let output = run(command.into_std(), &mut cancel)
            .await
            .context("run source build through mise")?;
        // These are explicit selected build diagnostics, not candidate probes.
        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        ensure!(
            output.status.success(),
            "source build failed before update publication"
        );
        root.revalidate()?;
    }
    if cancelled(&mut cancel) {
        return Err(Cancelled.into());
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| source.join("target"));
    let target = if target.is_absolute() {
        target
    } else {
        source.join(target)
    };
    Ok(target.join(host).join("release/kuru"))
}

async fn run(
    command: std::process::Command,
    cancel: &mut oneshot::Receiver<()>,
) -> Result<std::process::Output> {
    let mut owner = OwnedProcessGroup::spawn(
        command,
        StdioPlan::new(StdioSlot::Null, StdioSlot::Pipe, StdioSlot::Pipe),
    )?;
    let mut stdout = None;
    let mut stderr = None;
    // Every failure after spawn goes through the same retained cleanup owner.
    let body: Result<()> = async {
        stdout = Some(Capture::new(tokio::process::ChildStdout::from_std(owner.take_stdout()?)?));
        stderr = Some(Capture::new(tokio::process::ChildStderr::from_std(owner.take_stderr()?)?));
        let out = stdout.as_mut().expect("captured stdout");
        let err = stderr.as_mut().expect("captured stderr");
        let deadline = Instant::now() + BUILD_TIMEOUT;
        loop {
            match owner.root_state() {
                RootState::Exited => return Ok(()),
                RootState::Disarmed(reason) => anyhow::bail!("source build root ownership lost: {reason:?}"),
                RootState::Reaped(_) => anyhow::bail!("source build was reaped before cleanup"),
                RootState::Running | RootState::Interrupted => {}
            }
            tokio::select! {
                biased;
                _ = &mut *cancel => return Err(Cancelled.into()),
                _ = tokio::time::sleep_until(deadline) => anyhow::bail!("source build timed out before publication"),
                read = out.read(), if !out.eof => read?,
                read = err.read(), if !err.eof => read?,
                _ = tokio::time::sleep(POLL) => {}
            }
        }
    }.await;
    let cleanup = settle(&mut owner, &mut stdout, &mut stderr).await;
    let status = match (body, cleanup) {
        (Ok(()), Ok(status)) => status,
        (Err(error), Ok(_)) => return Err(error),
        (Ok(()), Err(error)) => return Err(error),
        (Err(error), Err(cleanup)) => {
            anyhow::bail!("{error:#}; source build cleanup failed: {cleanup:#}")
        }
    };
    let stdout = stdout.context("source build stdout unavailable after cleanup")?;
    let mut stderr = stderr.context("source build stderr unavailable after cleanup")?;
    if stdout.omitted || stderr.omitted {
        stderr
            .bytes
            .extend_from_slice(b"\nAdditional source build output omitted.\n");
    }
    Ok(std::process::Output {
        status,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

async fn settle(
    owner: &mut OwnedProcessGroup,
    stdout: &mut Option<Capture<tokio::process::ChildStdout>>,
    stderr: &mut Option<Capture<tokio::process::ChildStderr>>,
) -> Result<std::process::ExitStatus> {
    // This remains the operational failure deadline, not permission to drop
    // an unreaped owner. After expiry the SAME owner and pipe captures continue
    // to settle; no fresh signal authority or second operation budget is made.
    let deadline = Instant::now() + CLEANUP_TIMEOUT;
    let mut failure = None;
    loop {
        match owner.terminate_before_reap() {
            Termination::Signalled(_) | Termination::InvalidPhase => break,
            Termination::Interrupted => {}
            Termination::Disarmed(reason) => {
                failure.get_or_insert_with(|| {
                    anyhow::anyhow!("source build cleanup ownership lost: {reason:?}")
                });
                break;
            }
        }
        if Instant::now() >= deadline {
            failure.get_or_insert_with(|| {
                anyhow::anyhow!(
                    "source build cleanup could not signal owned root before its deadline"
                )
            });
        }
        tokio::time::sleep(POLL).await;
    }
    let mut status = None;
    let mut listing = owner.permission_listing(deadline.into_std());
    loop {
        if status.is_none() {
            match owner.pre_reap_step(deadline.into_std()) {
                PreReap::Ready | PreReap::Reaped | PreReap::Expired => match owner.reap_if_exited()
                {
                    Reap::Reaped(reaped) => status = Some(reaped),
                    Reap::NotExited | Reap::Interrupted => {}
                    other => {
                        failure.get_or_insert_with(|| {
                            anyhow::anyhow!("source build root reap refused: {other:?}")
                        });
                    }
                },
                PreReap::Pending | PreReap::ExpiredPending | PreReap::Unobserved(_) => {}
                other => {
                    failure.get_or_insert_with(|| {
                        anyhow::anyhow!("source build pre-reap refused: {other:?}")
                    });
                }
            }
        }
        let gone = if status.is_some() {
            match listing.resolve(owner.presence_after_reap()).await {
                GroupPresence::Absent | GroupPresence::Recycled => true,
                GroupPresence::Present | GroupPresence::PermissionDenied => false,
                other => {
                    failure.get_or_insert_with(|| {
                        anyhow::anyhow!("source build group cleanup unconfirmed: {other:?}")
                    });
                    false
                }
            }
        } else {
            false
        };
        if gone
            && stdout.as_ref().is_none_or(|out| out.eof || out.failed)
            && stderr.as_ref().is_none_or(|err| err.eof || err.failed)
        {
            // A failed reader grants no EOF proof or successful result. Its
            // retained pipe is closed only after actual root/group settlement.
            if let Some(error) = failure {
                return Err(error);
            }
            return status.context("source build root was not reaped");
        }
        if Instant::now() >= deadline {
            failure.get_or_insert_with(|| {
                anyhow::anyhow!("source build cleanup or pipe EOF unconfirmed before its deadline")
            });
        }
        tokio::select! {
            read = async { stdout.as_mut().expect("stdout exists").read().await }, if stdout.as_ref().is_some_and(|out| !out.eof && !out.failed) => {
                if let Err(error) = read {
                    stdout.as_mut().expect("stdout exists").failed = true;
                    failure.get_or_insert_with(|| anyhow::Error::new(error).context("source stdout EOF could not be confirmed"));
                }
            },
            read = async { stderr.as_mut().expect("stderr exists").read().await }, if stderr.as_ref().is_some_and(|err| !err.eof && !err.failed) => {
                if let Err(error) = read {
                    stderr.as_mut().expect("stderr exists").failed = true;
                    failure.get_or_insert_with(|| anyhow::Error::new(error).context("source stderr EOF could not be confirmed"));
                }
            },
            _ = tokio::time::sleep(POLL) => {}
        }
    }
}

/// Same concrete owned launch/capture used by native source update; available
/// only to explicitly built maintainer fixtures, not ordinary app dispatch.
#[cfg(feature = "tooling")]
pub mod test_support {
    pub async fn output(command: std::process::Command) -> anyhow::Result<std::process::Output> {
        let (_held, mut cancellation) = tokio::sync::oneshot::channel();
        super::run(command, &mut cancellation).await
    }
}
