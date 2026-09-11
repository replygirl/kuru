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

#[cfg(windows)]
pub use windows::Command;
#[cfg(windows)]
pub fn program(command: &Command) -> &std::ffi::OsStr {
    &command.program
}

#[cfg(windows)]
mod windows {
    use kuru_platform::windows::process::{
        NativeChild, Stdio, configured_command, environment_key_eq,
    };
    use std::{
        ffi::{OsStr, OsString},
        io,
        path::{Path, PathBuf},
        process::Output,
        time::{Duration, Instant},
    };
    use tokio::io::AsyncReadExt;

    const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;

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
            let mut stdout_bytes = Vec::new();
            let mut stderr_bytes = Vec::new();
            let mut stdout_eof = false;
            let mut stderr_eof = false;
            let mut phase = "read native stdout/stderr";
            let captured = tokio::time::timeout(timeout, async {
                tokio::try_join!(
                    read(&mut stdout, &mut stdout_bytes, &mut stdout_eof),
                    read(&mut stderr, &mut stderr_bytes, &mut stderr_eof)
                )?;
                phase = "wait for native process tree quiescence";
                child.wait(Duration::from_secs(5)).await
            })
            .await;
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
            Err(io::Error::new(
                error.kind(),
                format!(
                    "{phase} after {} ms: {error}; stdout_eof={stdout_eof} stderr_eof={stderr_eof}; \
                 cleanup={cleanup:?} stdout_close={stdout_close:?} stderr_close={stderr_close:?}; \
                 stdout prefix: {}; stderr prefix: {}",
                    elapsed.as_millis(),
                    prefix(&stdout_bytes),
                    prefix(&stderr_bytes)
                ),
            ))
        }
    }

    fn prefix(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024)])
    }

    async fn read(
        pipe: &mut kuru_platform::windows::pipe::Pipe,
        bytes: &mut Vec<u8>,
        eof: &mut bool,
    ) -> io::Result<()> {
        (&mut *pipe)
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(bytes)
            .await?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
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
