//! Maintainer command intent. Windows uses the same owned native process
//! boundary as the application; Unix retains its established Tokio command API.

/// Select a checkout independently of inherited Git repository selectors.
/// Apply deliberate overrides, such as a private index, after this call.
/// Global signing and authentication settings remain available to build tools.
pub fn rooted(root: &std::path::Path, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.current_dir(root);
    // Git's `rev-parse --local-env-vars` contract also applies to indirect Git
    // users such as mise, cog and Communiqué.
    for key in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_OBJECT_DIRECTORY",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_REPLACE_REF_BASE",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(key);
    }
    command
}

#[cfg(unix)]
pub use tokio::process::Command;
#[cfg(unix)]
pub fn program(command: &Command) -> &std::ffi::OsStr {
    command.as_std().get_program()
}

#[cfg(unix)]
pub use std::process::Command as BlockingCommand;

/// Synchronous fixture/maintainer callers still use atomic native ownership.
/// The worker owns a separate runtime so this facade can be called from a test
/// already running Tokio without nesting that runtime or using legacy spawn.
#[cfg(windows)]
pub struct BlockingCommand(Command);

#[cfg(windows)]
impl BlockingCommand {
    pub fn new(program: impl AsRef<std::ffi::OsStr>) -> Self {
        Self(Command::new(program))
    }
    pub fn arg(&mut self, value: impl AsRef<std::ffi::OsStr>) -> &mut Self {
        self.0.arg(value);
        self
    }
    pub fn args<I, S>(&mut self, values: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.0.args(values);
        self
    }
    pub fn current_dir(&mut self, path: impl AsRef<std::path::Path>) -> &mut Self {
        self.0.current_dir(path);
        self
    }
    pub fn env(
        &mut self,
        name: impl AsRef<std::ffi::OsStr>,
        value: impl AsRef<std::ffi::OsStr>,
    ) -> &mut Self {
        self.0.env(name, value);
        self
    }
    pub fn envs<I, K, V>(&mut self, values: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        for (name, value) in values {
            self.env(name, value);
        }
        self
    }
    pub fn env_remove(&mut self, name: impl AsRef<std::ffi::OsStr>) -> &mut Self {
        self.0.env_remove(name);
        self
    }
    pub fn env_clear(&mut self) -> &mut Self {
        self.0.env_clear();
        self
    }
    pub fn output(&mut self) -> std::io::Result<std::process::Output> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?
                        .block_on(self.0.output())
                })
                .join()
                .map_err(|_| std::io::Error::other("native command worker panicked"))?
        })
    }
    pub fn status(&mut self) -> std::io::Result<std::process::ExitStatus> {
        Ok(self.output()?.status)
    }
}

pub async fn output(
    command: &mut Command,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    #[cfg(unix)]
    return tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "tool timed out"))?;
    #[cfg(windows)]
    command.output_with_timeout(timeout).await
}

/// Run a bounded maintainer subprocess while draining both pipes and reaping its
/// owned Unix process group.
///
/// Delivery's advisory scanner uses this instead of Tokio's unbounded command
/// output collection. It is deliberately separate from [`output`], whose
/// established callers retain their existing command semantics.
#[cfg(feature = "tooling")]
pub async fn bounded_output(
    command: &mut Command,
    timeout: std::time::Duration,
    limit: usize,
) -> std::io::Result<std::process::Output> {
    #[cfg(unix)]
    return bounded_unix::output(command, timeout, limit).await;
    #[cfg(windows)]
    command
        .output_with_limit_and_timeout(timeout, limit as u64)
        .await
}

#[cfg(all(unix, feature = "tooling"))]
mod bounded_unix {
    use rustix::{
        io::Errno,
        process::{
            Pid, Signal, WaitId, WaitIdOptions, kill_process_group, test_kill_process_group, waitid,
        },
    };
    use std::{io, process::Stdio, time::Duration};
    use tokio::{
        io::{AsyncRead, AsyncReadExt},
        process::{Child, ChildStderr, ChildStdout},
        time::{Instant, MissedTickBehavior},
    };

    const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

    struct Capture<R> {
        reader: Option<R>,
        bytes: Vec<u8>,
        limit: usize,
    }

    impl<R: AsyncRead + Unpin> Capture<R> {
        fn new(reader: R, limit: usize) -> Self {
            Self {
                reader: Some(reader),
                bytes: Vec::new(),
                limit,
            }
        }

        fn eof(&self) -> bool {
            self.reader.is_none()
        }

        async fn read(&mut self) -> io::Result<()> {
            let mut chunk = [0; 8192];
            let length = self
                .reader
                .as_mut()
                .expect("capture is only read while active")
                .read(&mut chunk)
                .await?;
            if length == 0 {
                self.reader = None;
                return Ok(());
            }
            let retained = length.min(self.limit.saturating_sub(self.bytes.len()));
            self.bytes.extend_from_slice(&chunk[..retained]);
            if retained != length {
                return Err(io::Error::other("tool output exceeds limit"));
            }
            Ok(())
        }
    }

    fn root_exited(pid: Pid) -> io::Result<bool> {
        match waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(None) => Ok(false),
            Ok(Some(_)) => Ok(true),
            Err(error) => Err(io::Error::other(format!(
                "lost owned process identity before cleanup: {error}"
            ))),
        }
    }

    async fn wait_for_group_gone_with(
        mut observe: impl FnMut() -> Result<(), Errno>,
    ) -> io::Result<()> {
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        loop {
            let permission_pending = match observe() {
                Err(Errno::SRCH) => return Ok(()),
                Ok(()) => false,
                Err(Errno::PERM) => true,
                Err(error) => {
                    return Err(io::Error::other(format!(
                        "post-cleanup process-group query: {error}"
                    )));
                }
            };
            if Instant::now() >= deadline {
                if permission_pending {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "post-cleanup process-group permission persisted through cleanup deadline: EPERM",
                    ));
                }
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "owned process group survived cleanup",
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_group_gone(pid: Pid) -> io::Result<()> {
        wait_for_group_gone_with(|| test_kill_process_group(pid)).await
    }

    async fn stop_and_reap(child: &mut Child, pid: Pid) -> String {
        let mut issues = Vec::new();
        match root_exited(pid) {
            Ok(_) => match kill_process_group(pid, Signal::KILL) {
                Ok(()) | Err(Errno::SRCH) => {}
                Err(error) => issues.push(format!("owned process-group termination: {error}")),
            },
            Err(error) => issues.push(error.to_string()),
        }
        match tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => issues.push(format!("root reap: {error}")),
            Err(_) => issues.push("root reap timed out".to_owned()),
        }
        if let Err(error) = wait_for_group_gone(pid).await {
            issues.push(error.to_string());
        }
        if issues.is_empty() {
            "owned process group stopped and root reaped".to_owned()
        } else {
            format!("cleanup issues: {issues:?}")
        }
    }

    async fn finish_and_reap(child: &mut Child, pid: Pid) -> io::Result<std::process::ExitStatus> {
        ensure_root_exited(pid)?;
        let mut issues = Vec::new();
        // The root is still an unreaped waitid anchor here. Terminate any
        // helper that inherited its fresh group before that identity can be
        // reused, even when the root itself reported success and both pipes
        // were already closed.
        let termination = match kill_process_group(pid, Signal::KILL) {
            Ok(()) | Err(Errno::SRCH) => None,
            Err(error) => Some(error),
        };
        let status = match tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                return Err(io::Error::other(format!(
                    "root reap after natural exit: {error}"
                )));
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "root reap timed out",
                ));
            }
        };
        match wait_for_group_gone(pid).await {
            Ok(()) => {
                // macOS can reject signalling a group whose only member is an
                // already-exited root. Successful post-reap absence proves no
                // helper survived; it does not treat EPERM as ownership proof.
            }
            Err(error) => {
                if let Some(termination) = termination {
                    issues.push(format!("owned process-group termination: {termination}"));
                }
                issues.push(error.to_string());
            }
        }
        if issues.is_empty() {
            Ok(status)
        } else {
            Err(io::Error::other(format!(
                "natural process-group cleanup issues: {issues:?}"
            )))
        }
    }

    async fn capture_until(
        pid: Pid,
        stdout: &mut Capture<ChildStdout>,
        stderr: &mut Capture<ChildStderr>,
        deadline: Instant,
    ) -> io::Result<()> {
        // Pipes can close before a command performs its final exit. Keep the
        // root as an unreaped waitid anchor, but poll that state until it exits
        // instead of waiting until the overall deadline with both readers idle.
        let mut observation = tokio::time::interval(Duration::from_millis(10));
        observation.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            if root_exited(pid)? && stdout.eof() && stderr.eof() {
                return Ok(());
            }
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "tool timed out"));
                }
                result = stdout.read(), if !stdout.eof() => result?,
                result = stderr.read(), if !stderr.eof() => result?,
                _ = observation.tick() => {}
            }
        }
    }

    pub(super) async fn output(
        command: &mut super::Command,
        timeout: Duration,
        limit: usize,
    ) -> io::Result<std::process::Output> {
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(false);
        let mut child = command.spawn()?;
        let pid = Pid::from_raw(
            child
                .id()
                .ok_or_else(|| io::Error::other("owned tool did not expose a process ID"))?
                .try_into()
                .map_err(|_| io::Error::other("owned tool process ID did not fit native range"))?,
        )
        .ok_or_else(|| io::Error::other("owned tool process ID was invalid"))?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let cleanup = stop_and_reap(&mut child, pid).await;
                return Err(io::Error::other(format!("missing tool stdout; {cleanup}")));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                let cleanup = stop_and_reap(&mut child, pid).await;
                return Err(io::Error::other(format!("missing tool stderr; {cleanup}")));
            }
        };
        let mut stdout = Capture::new(stdout, limit);
        let mut stderr = Capture::new(stderr, limit);
        if let Err(error) =
            capture_until(pid, &mut stdout, &mut stderr, Instant::now() + timeout).await
        {
            let cleanup = stop_and_reap(&mut child, pid).await;
            return Err(io::Error::new(error.kind(), format!("{error}; {cleanup}")));
        }
        let status = finish_and_reap(&mut child, pid).await?;
        Ok(std::process::Output {
            status,
            stdout: stdout.bytes,
            stderr: stderr.bytes,
        })
    }

    fn ensure_root_exited(pid: Pid) -> io::Result<()> {
        if root_exited(pid)? {
            Ok(())
        } else {
            Err(io::Error::other(
                "owned root was still running after both output pipes closed",
            ))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::VecDeque;

        #[tokio::test]
        async fn permission_then_absence_waits_for_observed_group_absence() {
            let mut observed = VecDeque::from([Err(Errno::PERM), Err(Errno::SRCH)]);
            wait_for_group_gone_with(|| observed.pop_front().expect("observer call"))
                .await
                .unwrap();
            assert!(observed.is_empty(), "observer must wait for ESRCH");
        }

        #[tokio::test]
        async fn persistent_permission_remains_a_bounded_error() {
            let started = Instant::now();
            let mut observed = 0usize;
            let error = wait_for_group_gone_with(|| {
                observed += 1;
                Err(Errno::PERM)
            })
            .await
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert!(
                error
                    .to_string()
                    .contains("permission persisted through cleanup deadline: EPERM"),
                "{error}"
            );
            assert!(started.elapsed() >= CLEANUP_TIMEOUT);
            assert!(
                started.elapsed() < CLEANUP_TIMEOUT + Duration::from_secs(1),
                "permission observation exceeded its fixed cleanup deadline"
            );
            assert!(observed > 1, "permission must be observed more than once");
        }

        #[tokio::test]
        async fn existing_group_then_absence_preserves_success() {
            let mut observed = VecDeque::from([Ok(()), Err(Errno::SRCH)]);
            wait_for_group_gone_with(|| observed.pop_front().expect("observer call"))
                .await
                .unwrap();
            assert!(observed.is_empty());
        }

        #[tokio::test]
        async fn unexpected_group_observation_preserves_failure() {
            let error = wait_for_group_gone_with(|| Err(Errno::INVAL))
                .await
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("post-cleanup process-group query: Invalid argument"),
                "{error}"
            );
        }
    }
}

#[cfg(windows)]
pub use windows::Command;
#[cfg(windows)]
pub fn program(command: &Command) -> &std::ffi::OsStr {
    &command.program
}

#[cfg(windows)]
mod windows {
    use kuru_platform::windows::process::{
        NativeChild, ProcessSample, Stdio, configured_command, environment_key_eq, sample_process,
    };
    use std::{
        ffi::{OsStr, OsString},
        fmt::Write as _,
        io,
        os::windows::io::OwnedHandle,
        path::{Path, PathBuf},
        process::Output,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use tokio::io::AsyncReadExt;

    /// Interval between diagnostic CPU-time/working-set samples while a
    /// native command is still within its timeout window. Diagnostics only;
    /// never affects the success path or the timeout bound itself.
    const SAMPLE_INTERVAL: Duration = Duration::from_secs(10);

    pub(super) const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;

    pub struct Command {
        pub(super) program: OsString,
        arguments: Vec<OsString>,
        directory: Option<PathBuf>,
        environment: Vec<(OsString, OsString)>,
    }

    impl Command {
        pub fn new(program: impl AsRef<OsStr>) -> Self {
            Self {
                program: program.as_ref().to_owned(),
                arguments: Vec::new(),
                directory: None,
                environment: std::env::vars_os().collect(),
            }
        }
        pub fn arg(&mut self, value: impl AsRef<OsStr>) -> &mut Self {
            self.arguments.push(value.as_ref().to_owned());
            self
        }
        pub fn args<I, S>(&mut self, values: I) -> &mut Self
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            self.arguments
                .extend(values.into_iter().map(|value| value.as_ref().to_owned()));
            self
        }
        pub fn current_dir(&mut self, path: impl AsRef<Path>) -> &mut Self {
            self.directory = Some(path.as_ref().to_owned());
            self
        }
        pub fn env(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
            self.env_remove(name.as_ref());
            self.environment
                .push((name.as_ref().to_owned(), value.as_ref().to_owned()));
            self
        }
        pub fn envs<I, K, V>(&mut self, values: I) -> &mut Self
        where
            I: IntoIterator<Item = (K, V)>,
            K: AsRef<OsStr>,
            V: AsRef<OsStr>,
        {
            for (name, value) in values {
                self.env(name, value);
            }
            self
        }
        pub fn env_remove(&mut self, name: impl AsRef<OsStr>) -> &mut Self {
            self.environment
                .retain(|(key, _)| !environment_key_eq(key, name.as_ref()));
            self
        }
        pub fn env_clear(&mut self) -> &mut Self {
            self.environment.clear();
            self
        }
        /// The native boundary always owns and terminates its process tree.
        pub fn kill_on_drop(&mut self, _: bool) -> &mut Self {
            self
        }

        pub async fn output(&mut self) -> io::Result<Output> {
            self.output_with_timeout(Duration::from_secs(180)).await
        }

        pub(super) async fn output_with_timeout(
            &mut self,
            timeout: Duration,
        ) -> io::Result<Output> {
            self.output_with_limit_and_timeout(timeout, OUTPUT_LIMIT)
                .await
        }

        pub(super) async fn output_with_limit_and_timeout(
            &mut self,
            timeout: Duration,
            limit: u64,
        ) -> io::Result<Output> {
            if limit > OUTPUT_LIMIT {
                return Err(io::Error::other("requested output limit is too large"));
            }
            let cwd = self.directory.clone().unwrap_or(std::env::current_dir()?);
            let cwd = if cwd.is_absolute() {
                cwd
            } else {
                std::env::current_dir()?.join(cwd)
            };
            let mut spec = configured_command(
                &self.program,
                &self.arguments,
                &cwd,
                self.environment.clone(),
            )?;
            spec.stdout = Stdio::Pipe;
            spec.stderr = Stdio::Pipe;
            let mut child = spec.spawn().await?;
            let mut stdout = child
                .take_stdout()
                .ok_or_else(|| io::Error::other("missing native stdout"))?;
            let mut stderr = child
                .take_stderr()
                .ok_or_else(|| io::Error::other("missing native stderr"))?;
            let started = Instant::now();
            // Diagnostics-only duplicate: absence (e.g. denied query rights)
            // must never affect the timed read/wait below, only the trace
            // attached to a subsequent timeout's error text.
            let diagnostic_handle = child.duplicate_diagnostic_handle().ok();
            let trace: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let sampler =
                diagnostic_handle.map(|handle| spawn_sampler(handle, started, Arc::clone(&trace)));
            let mut stdout_bytes = Vec::new();
            let mut stderr_bytes = Vec::new();
            let mut stdout_eof = false;
            let mut stderr_eof = false;
            let mut phase = "read native stdout/stderr";
            let captured = tokio::time::timeout(timeout, async {
                tokio::try_join!(
                    read(&mut stdout, &mut stdout_bytes, &mut stdout_eof, limit),
                    read(&mut stderr, &mut stderr_bytes, &mut stderr_eof, limit)
                )?;
                phase = "wait for native process tree quiescence";
                child.wait(Duration::from_secs(5)).await
            })
            .await;
            // The trace is diagnostic-only and only ever read on the failure
            // path below; stop sampling as soon as the timed future settles,
            // on both success and failure, so no sampler task outlives this call.
            if let Some(sampler) = sampler {
                sampler.abort();
            }
            let error = match captured {
                Ok(Ok(status)) => {
                    return Ok(Output {
                        status,
                        stdout: stdout_bytes,
                        stderr: stderr_bytes,
                    });
                }
                Ok(Err(error)) => error,
                Err(_) => io::Error::new(io::ErrorKind::TimedOut, "tool timed out"),
            };
            // Keep partial/completed output outside the timed future, including
            // a root's final failure when an enclosing Job still has a live
            // trusted helper. Cleanup failure must not replace that first cause.
            let elapsed = started.elapsed();
            let cleanup = terminate(&mut child).await;
            let (stdout_close, stderr_close) = tokio::join!(
                stdout.close(Duration::from_secs(5)),
                stderr.close(Duration::from_secs(5))
            );
            let samples = trace
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Err(io::Error::new(
                error.kind(),
                format!(
                    "{phase} after {} ms: {error}; stdout_eof={stdout_eof} stderr_eof={stderr_eof}; \
                 cleanup={cleanup:?} stdout_close={stdout_close:?} stderr_close={stderr_close:?}; \
                 samples=[{}]; stdout prefix: {}; stderr prefix: {}",
                    elapsed.as_millis(),
                    samples.join(", "),
                    prefix(&stdout_bytes),
                    prefix(&stderr_bytes)
                ),
            ))
        }
    }

    /// Poll CPU time and working set roughly every [`SAMPLE_INTERVAL`] and
    /// append a formatted point to `trace`. Purely diagnostic: a sample
    /// failure is recorded as text, never returned as an error, and the
    /// caller aborts this task as soon as the timed operation settles.
    fn spawn_sampler(
        handle: OwnedHandle,
        started: Instant,
        trace: Arc<Mutex<Vec<String>>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SAMPLE_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await; // the first tick fires immediately; skip it
            loop {
                ticker.tick().await;
                let mut point = format!("elapsed={}ms", started.elapsed().as_millis());
                match sample_process(&handle) {
                    Ok(ProcessSample {
                        kernel_time,
                        user_time,
                        working_set_bytes,
                    }) => {
                        let _ = write!(
                            point,
                            " cpu={}ms working_set={working_set_bytes}B",
                            (kernel_time + user_time).as_millis()
                        );
                    }
                    Err(error) => {
                        let _ = write!(point, " sample_error={error}");
                    }
                }
                if let Ok(mut trace) = trace.lock() {
                    trace.push(point);
                }
            }
        })
    }

    fn prefix(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024)])
    }

    async fn read(
        pipe: &mut kuru_platform::windows::pipe::Pipe,
        bytes: &mut Vec<u8>,
        eof: &mut bool,
        limit: u64,
    ) -> io::Result<()> {
        (&mut *pipe).take(limit + 1).read_to_end(bytes).await?;
        if bytes.len() as u64 > limit {
            return Err(io::Error::other("tool output exceeds limit"));
        }
        *eof = true;
        pipe.close(Duration::from_secs(5)).await
    }

    async fn terminate(child: &mut NativeChild) -> io::Result<()> {
        let termination = child.terminate();
        let reaped = child.wait(Duration::from_secs(5)).await;
        match (termination, reaped) {
            (Ok(()), Ok(_)) => Ok(()),
            (termination, reaped) => Err(io::Error::other(format!(
                "native cleanup: terminate={termination:?}; wait={reaped:?}"
            ))),
        }
    }
}
