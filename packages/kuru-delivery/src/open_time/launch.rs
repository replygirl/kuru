//! Run the measured command with streamed, timestamped stderr lines.
//!
//! The command's own descendants (the memory owner, supervisors and engines)
//! are independent services; on Windows the command runs in a fixture Job
//! that permits exactly their explicit breakaway, as Kuru's native fixtures do.

use std::{
    ffi::OsString,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};

use super::{Line, observe::millis};

/// Bytes of stdout kept to confirm JSON output; the rest is drained.
const STDOUT_KEPT: usize = 64 * 1024;
/// Longest stderr line retained.
const LINE_LIMIT: usize = 512;
/// Most stderr lines retained.
const LINES_KEPT: usize = 64;

pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub environment: Vec<(OsString, OsString)>,
    pub cwd: PathBuf,
}

#[derive(Debug, Default)]
pub struct Finished {
    pub spawn_ms: f64,
    pub lines: Vec<Line>,
    pub stdout: Vec<u8>,
    pub exit_ms: Option<f64>,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
}

/// Read lines as they arrive, keeping each with the time it was read. The
/// lines already kept survive a timeout of the enclosing future.
async fn lines(
    stream: impl AsyncRead + Unpin,
    epoch: Instant,
    kept: &Mutex<Vec<Line>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        if reader.read_until(b'\n', &mut buffer).await? == 0 {
            return Ok(());
        }
        let t_ms = millis(epoch.elapsed());
        let mut kept = kept
            .lock()
            .map_err(|_| std::io::Error::other("line store poisoned"))?;
        if kept.len() < LINES_KEPT {
            let text = String::from_utf8_lossy(&buffer);
            let text = text.trim_end_matches(['\r', '\n']);
            kept.push(Line {
                t_ms,
                text: text.chars().take(LINE_LIMIT).collect(),
            });
        }
    }
}

fn taken(kept: Mutex<Vec<Line>>) -> Vec<Line> {
    kept.into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn drain(stream: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut kept = Vec::new();
    let mut reader = stream;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(kept);
        }
        let room = STDOUT_KEPT.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..read.min(room)]);
    }
}

/// Run to completion within `bound`. Past the bound the command's own tree is
/// terminated through its retained child handle and the run is marked
/// `timed_out`; the memory services it started are left to retire.
#[cfg(unix)]
pub async fn run(launch: &Launch, epoch: Instant, bound: Duration) -> Result<Finished> {
    use std::process::Stdio;
    let mut command = tokio::process::Command::new(&launch.program);
    command
        .args(&launch.args)
        .env_clear()
        .envs(launch.environment.iter().map(|(key, value)| (key, value)))
        .current_dir(&launch.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .with_context(|| format!("start {}", launch.program.display()))?;
    let spawn_ms = millis(epoch.elapsed());
    let stdout = child.stdout.take().context("missing stdout pipe")?;
    let stderr = child.stderr.take().context("missing stderr pipe")?;
    let kept = Mutex::new(Vec::new());
    let completed = tokio::time::timeout(bound, async {
        tokio::join!(lines(stderr, epoch, &kept), drain(stdout), async {
            let status = child.wait().await;
            (status, millis(epoch.elapsed()))
        })
    })
    .await;
    match completed {
        Ok((read, stdout, (status, exit_ms))) => {
            let status = status.context("wait for the measured command")?;
            read.context("read stderr")?;
            Ok(Finished {
                spawn_ms,
                lines: taken(kept),
                stdout: stdout.context("read stdout")?,
                exit_ms: Some(exit_ms),
                exit_code: status.code(),
                success: status.success(),
                timed_out: false,
            })
        }
        Err(_) => {
            child.start_kill().context("stop the timed-out command")?;
            let status = child.wait().await.context("reap the timed-out command")?;
            Ok(Finished {
                spawn_ms,
                lines: taken(kept),
                exit_code: status.code(),
                timed_out: true,
                ..Finished::default()
            })
        }
    }
}

#[cfg(windows)]
pub async fn run(launch: &Launch, epoch: Instant, bound: Duration) -> Result<Finished> {
    use kuru_platform::windows::process::{Lifetime, Stdio, configured_command};
    let mut spec = configured_command(
        launch.program.as_os_str(),
        &launch.args,
        &launch.cwd,
        launch.environment.clone(),
    )
    .with_context(|| format!("prepare {}", launch.program.display()))?;
    spec.lifetime = Lifetime::FixtureBreakawayJob;
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    let mut child = spec
        .spawn()
        .await
        .with_context(|| format!("start {}", launch.program.display()))?;
    let spawn_ms = millis(epoch.elapsed());
    let mut stdout = child.take_stdout().context("missing stdout pipe")?;
    let mut stderr = child.take_stderr().context("missing stderr pipe")?;
    let kept = Mutex::new(Vec::new());
    let completed = tokio::time::timeout(bound, async {
        let (read, stdout) = tokio::join!(lines(&mut stderr, epoch, &kept), drain(&mut stdout));
        // Both pipes are at EOF, so the root has exited or closed them; the
        // wait proves the owned Job is quiescent.
        let status = child.wait(Duration::from_secs(10)).await;
        (read, stdout, status, millis(epoch.elapsed()))
    })
    .await;
    match completed {
        Ok((read, stdout, status, exit_ms)) => {
            let status = status.context("wait for the measured command")?;
            read.context("read stderr")?;
            Ok(Finished {
                spawn_ms,
                lines: taken(kept),
                stdout: stdout.context("read stdout")?,
                exit_ms: Some(exit_ms),
                exit_code: status.code(),
                success: status.success(),
                timed_out: false,
            })
        }
        Err(_) => {
            child.terminate().context("stop the timed-out command")?;
            let status = child
                .wait(Duration::from_secs(10))
                .await
                .context("reap the timed-out command")?;
            Ok(Finished {
                spawn_ms,
                lines: taken(kept),
                exit_code: status.code(),
                timed_out: true,
                ..Finished::default()
            })
        }
    }
}
