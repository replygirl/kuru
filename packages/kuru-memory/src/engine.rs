//! Small process adapter; SQL supervision and its deadlines remain shared.
use anyhow::{Context, Result};
use std::{ffi::OsString, io, path::Path, process::ExitStatus, time::Duration};

#[cfg(unix)]
type Native = tokio::process::Child;
#[cfg(windows)]
type Native = kuru_platform::windows::process::NativeChild;

pub(crate) struct Child {
    inner: Native,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopOutcome {
    AlreadyExited,
    Graceful,
    Forced,
}

pub(crate) async fn spawn(
    binary: &Path,
    home: &Path,
    cwd: &Path,
    arguments: Vec<OsString>,
    extra: Vec<(OsString, OsString)>,
    server: bool,
) -> Result<Child> {
    #[cfg(unix)]
    {
        let _ = server;
        let mut command = crate::provision::isolated_command(binary, home);
        command
            .args(arguments)
            .envs(extra)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        Ok(Child {
            inner: command.spawn().context("start verified Dolt engine")?,
        })
    }
    #[cfg(windows)]
    {
        use kuru_platform::windows::process::{Console, NativeSpawnSpec, Stdio};
        let mut command = NativeSpawnSpec::new(binary.to_owned(), cwd.to_owned());
        command.args = arguments;
        command.environment = environment(home)?;
        command.environment.extend(extra);
        command.stdout = Stdio::Pipe;
        command.stderr = Stdio::Pipe;
        if server {
            command.console = Console::NewProcessGroup;
        }
        Ok(Child {
            inner: command
                .spawn()
                .await
                .context("start verified Dolt engine")?,
        })
    }
}

#[cfg(windows)]
pub(crate) fn environment(home: &Path) -> Result<Vec<(OsString, OsString)>> {
    let system = kuru_platform::windows::process::system_directory()?;
    let root = system
        .parent()
        .context("Windows system directory has no parent")?;
    Ok(vec![
        ("SystemRoot".into(), root.as_os_str().into()),
        ("WINDIR".into(), root.as_os_str().into()),
        ("PATH".into(), system.as_os_str().into()),
        ("HOME".into(), home.join("home").into_os_string()),
        ("USERPROFILE".into(), home.join("home").into_os_string()),
        ("DOLT_ROOT_PATH".into(), home.join("root").into_os_string()),
        ("TMPDIR".into(), home.join("tmp").into_os_string()),
        ("TMP".into(), home.join("tmp").into_os_string()),
        ("TEMP".into(), home.join("tmp").into_os_string()),
        ("DOLT_DISABLE_EVENT_FLUSH".into(), "1".into()),
    ])
}

impl Child {
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    pub(crate) async fn wait(&mut self) -> io::Result<ExitStatus> {
        #[cfg(unix)]
        {
            self.inner.wait().await
        }
        #[cfg(windows)]
        loop {
            if let Some(status) = self.inner.try_wait()? {
                return Ok(status);
            }
            // NativeChild only reports completion after the Job is empty.
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[cfg(unix)]
    pub(crate) fn stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.inner.stdout.take()
    }
    #[cfg(unix)]
    pub(crate) fn stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.inner.stderr.take()
    }
    #[cfg(windows)]
    pub(crate) fn stdout(&mut self) -> Option<kuru_platform::windows::pipe::Pipe> {
        self.inner.take_stdout()
    }
    #[cfg(windows)]
    pub(crate) fn stderr(&mut self) -> Option<kuru_platform::windows::pipe::Pipe> {
        self.inner.take_stderr()
    }

    pub(crate) fn kill(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.inner.start_kill()
        }
        #[cfg(windows)]
        {
            self.inner.terminate()
        }
    }

    pub(crate) async fn stop(
        &mut self,
        grace: Duration,
        kill_grace: Duration,
    ) -> Result<StopOutcome> {
        if self.try_wait()?.is_some() {
            return Ok(StopOutcome::AlreadyExited);
        }
        #[cfg(unix)]
        {
            use nix::{
                sys::signal::{Signal, kill},
                unistd::Pid,
            };
            let pid = self
                .inner
                .id()
                .context("owned Dolt child has no PID")?
                .try_into()?;
            match kill(Pid::from_raw(pid), Signal::SIGTERM) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                Err(error) => return Err(error.into()),
            }
        }
        #[cfg(windows)]
        if self.inner.interrupt().is_err() {
            // A missing console must not strand a retained Job and store lease.
            // Forced shutdown still requires observed whole-tree completion.
            self.kill()?;
            tokio::time::timeout(kill_grace, self.wait())
                .await
                .context(
                    "owned Dolt tree did not become quiescent after failed console interruption",
                )??;
            return Ok(StopOutcome::Forced);
        }
        if let Ok(result) = tokio::time::timeout(grace, self.wait()).await {
            result?;
            return Ok(StopOutcome::Graceful);
        }
        self.kill()?;
        tokio::time::timeout(kill_grace, self.wait())
            .await
            .context("owned Dolt tree did not become quiescent after forced shutdown")??;
        Ok(StopOutcome::Forced)
    }
}
