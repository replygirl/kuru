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
        time::Duration,
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
            let stdout = child
                .take_stdout()
                .ok_or_else(|| io::Error::other("missing native stdout"))?;
            let stderr = child
                .take_stderr()
                .ok_or_else(|| io::Error::other("missing native stderr"))?;
            let captured = tokio::time::timeout(timeout, async {
                let (stdout, stderr) = tokio::try_join!(read(stdout), read(stderr))?;
                let status = child.wait(Duration::from_secs(5)).await?;
                Ok::<_, io::Error>(Output {
                    status,
                    stdout,
                    stderr,
                })
            })
            .await;
            match captured {
                Ok(Ok(output)) => Ok(output),
                Ok(Err(error)) => {
                    terminate(&mut child).await?;
                    Err(error)
                }
                Err(_) => {
                    terminate(&mut child).await?;
                    Err(io::Error::new(io::ErrorKind::TimedOut, "tool timed out"))
                }
            }
        }
    }

    async fn read(mut pipe: kuru_platform::windows::pipe::Pipe) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        (&mut pipe)
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .await?;
        pipe.close(Duration::from_secs(5)).await?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(io::Error::other("tool output exceeds limit"));
        }
        Ok(bytes)
    }

    async fn terminate(child: &mut NativeChild) -> io::Result<()> {
        child.terminate()?;
        child.wait(Duration::from_secs(5)).await?;
        Ok(())
    }
}
