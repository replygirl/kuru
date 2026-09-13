use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[cfg(unix)]
use crate::unix_shell::ShellRegistry;
use anyhow::{Context, Result, bail, ensure};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use kuru_core::{Config, ToolSpec};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
#[cfg(windows)]
use kuru_platform::fs::{regular_file_info, validate_component};
use serde_json::{Value, json};
#[cfg(all(unix, test))]
use std::process::Stdio;
use tokio::io::AsyncReadExt;
#[cfg(all(unix, test))]
use tokio::process::Command;
#[cfg(any(windows, test))]
use tokio::time::timeout;
#[cfg(windows)]
type PathGuard = Directory;
#[cfg(unix)]
type PathGuard = ();

#[cfg(unix)]
const UNIX_SHELL_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "TZ",
    "NO_COLOR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
];

#[cfg(windows)]
const WINDOWS_SHELL_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "USERNAME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_ARCHITEW6432",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "TZ",
    "NO_COLOR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "PATHEXT",
];

#[cfg(unix)]
fn unix_shell_environment(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let environment: Vec<_> = environment.into_iter().collect();
    UNIX_SHELL_ENVIRONMENT
        .iter()
        .filter_map(|name| {
            environment
                .iter()
                .find(|(key, _)| key == OsStr::new(name))
                .map(|(_, value)| (OsString::from(name), value.clone()))
        })
        .collect()
}

#[cfg(windows)]
fn windows_shell_environment(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
    system_directory: &Path,
) -> Result<Vec<(OsString, OsString)>> {
    use kuru_platform::windows::process::environment_key_eq;

    let environment: Vec<_> = environment.into_iter().collect();
    let is_allowed = |key: &OsStr| {
        WINDOWS_SHELL_ENVIRONMENT
            .iter()
            .any(|name| environment_key_eq(key, OsStr::new(name)))
    };
    for (index, (key, _)) in environment
        .iter()
        .enumerate()
        .filter(|(_, (key, _))| is_allowed(key))
    {
        ensure!(
            !environment[index + 1..]
                .iter()
                .any(|(other, _)| is_allowed(other) && environment_key_eq(key, other)),
            "ambiguous case-equivalent Windows environment key"
        );
    }
    let mut projected: Vec<_> = WINDOWS_SHELL_ENVIRONMENT
        .iter()
        .filter_map(|name| {
            environment
                .iter()
                .find(|(key, _)| environment_key_eq(key, OsStr::new(name)))
                .map(|(_, value)| (OsString::from(name), value.clone()))
        })
        .collect();
    if !projected
        .iter()
        .any(|(key, _)| environment_key_eq(key, OsStr::new("PATHEXT")))
    {
        projected.push(("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()));
    }
    let windows = system_directory
        .parent()
        .context("native system directory lacks Windows parent")?;
    projected.extend([
        ("SystemRoot".into(), windows.as_os_str().into()),
        ("WINDIR".into(), windows.as_os_str().into()),
        ("ComSpec".into(), system_directory.join("cmd.exe").into()),
    ]);
    Ok(projected)
}

use crate::{
    MAX_BYTES,
    mcp::{McpExecution, McpHosts, McpStatus},
    redaction,
    tool_output::{ProjectedToolError, ToolContent, ToolExecution, ToolFailure, ToolFailureKind},
};

/// File tools operate under an opened directory capability. Shell and MCP
/// authorization grant process/server authority; cwd is not an OS sandbox.
pub struct ToolHost {
    root: PathBuf,
    directory: Dir,
    root_guard: Arc<Directory>,
    allow_write: bool,
    allow_shell: bool,
    mcp: McpHosts,
    #[cfg(unix)]
    shells: ShellRegistry,
}

pub struct ToolCatalog {
    tools: Vec<ToolSpec>,
    mcp: Vec<McpStatus>,
}

impl ToolCatalog {
    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    pub fn mcp(&self) -> &[McpStatus] {
        &self.mcp
    }

    pub fn into_tools(self) -> Vec<ToolSpec> {
        self.tools
    }
}

impl ToolHost {
    pub fn new(root: &Path, config: &Config) -> Result<Self> {
        let root = root.canonicalize().context("tool root does not exist")?;
        let root_guard = Arc::new(Directory::open(
            &root,
            Privacy::Inherited,
            NameRetention::Pinned,
        )?);
        Self::with_retained_root(root_guard, config)
    }

    /// Construct a host from the workspace capability retained before
    /// configuration review. Configured process launches keep this exact guard.
    pub fn with_retained_root(root_guard: Arc<Directory>, config: &Config) -> Result<Self> {
        root_guard.revalidate()?;
        let root = root_guard.path().to_path_buf();
        let directory = Dir::open_ambient_dir(&root, ambient_authority())?;
        root_guard.revalidate()?;
        Ok(Self {
            mcp: McpHosts::with_retained_root(root_guard.clone(), &config.mcp)?,
            root,
            directory,
            root_guard,
            allow_write: config.allow_write,
            allow_shell: config.allow_shell,
            #[cfg(unix)]
            shells: ShellRegistry::new(),
        })
    }

    /// Verify the held workspace identity without reopening a replacement.
    pub fn revalidate_root(&self) -> Result<()> {
        Ok(self.root_guard.revalidate()?)
    }

    /// Canonical pathname paired with the retained root capability.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn specs(&self) -> Result<Vec<ToolSpec>> {
        Ok(self.catalog().await?.into_tools())
    }

    pub async fn catalog(&self) -> Result<ToolCatalog> {
        let mut specs = vec![
            spec(
                "file_read",
                "Read a UTF-8 project file with a bounded head-and-tail excerpt.",
                &["path"],
                &["path"],
            ),
            spec(
                "file_list",
                "List immediate children of a project directory. Protected paths are omitted.",
                &["path"],
                &[],
            ),
        ];
        if self.allow_write {
            specs.push(spec("file_write", "Create or replace a project file. Parent directories must exist. Instruction/config/memory paths are protected.", &["path", "content"], &["path", "content"]));
            specs.push(spec(
                "file_delete",
                "Delete a single project file (never directories).",
                &["path"],
                &["path"],
            ));
        }
        if self.allow_shell {
            let mut shell = spec(
                "shell",
                "Execute a shell command with process authority, in the project cwd. This is not a filesystem sandbox. Only the documented compatibility environment is inherited. Output is capped; timeout is at most 120 seconds.",
                &["command"],
                &["command"],
            );
            shell.parameters["properties"]["timeout_ms"] =
                json!({"type":"integer","minimum":1,"maximum":120000});
            specs.push(shell);
        }
        let mcp = self.mcp.catalog().await?;
        specs.extend(mcp.tools);
        Ok(ToolCatalog {
            tools: specs,
            mcp: mcp.statuses,
        })
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        match self.execute_inner(name, args).await {
            Ok(execution) => project_execution(execution),
            Err(failure) => project_failure(failure),
        }
    }

    async fn execute_inner(
        &self,
        name: &str,
        args: Value,
    ) -> std::result::Result<ToolExecution, ToolFailure> {
        if !args.is_object() {
            return Err(ToolFailure::built_in(anyhow::anyhow!(
                "tool arguments must be an object"
            )));
        }
        match name {
            "file_read" => {
                let execution = async {
                    let (directory, path, _guard) = self.path(string(&args, "path")?, false)?;
                    let mut options = OpenOptions::new();
                    options.read(true).follow(FollowSymlinks::No);
                    #[cfg(unix)]
                    {
                        use cap_std::fs::OpenOptionsExt;
                        options.custom_flags(nix::libc::O_NONBLOCK);
                    }
                    let file = directory.open_with(path, &options)?;
                    #[cfg(windows)]
                    let file = {
                        let file = file.into_std();
                        ensure!(
                            regular_file_info(&file)?.links == 1,
                            "hard-linked files are not readable through file tools"
                        );
                        file
                    };
                    ensure!(
                        file.metadata()?.is_file(),
                        "file_read requires a regular file"
                    );
                    #[cfg(unix)]
                    {
                        use cap_std::fs::MetadataExt;
                        ensure!(
                            file.metadata()?.nlink() == 1,
                            "hard-linked files are not readable through file tools"
                        );
                    }
                    // Bound the stream to the initially checked regular-file extent. A
                    // concurrent append cannot keep the scanner reading indefinitely.
                    let extent = file.metadata()?.len();
                    #[cfg(unix)]
                    let file = file.into_std();
                    Ok(ToolExecution::ProjectedText(
                        read_file_output(tokio::fs::File::from_std(file), extent).await?,
                    ))
                }
                .await;
                execution.map_err(ToolFailure::built_in)
            }
            "file_write" => {
                let execution = (|| -> Result<ToolExecution> {
                    ensure!(self.allow_write, "file writes require allow_write=true");
                    let content = string(&args, "content")?;
                    ensure!(
                        content.len() <= MAX_BYTES,
                        "file content exceeds 2 MiB limit"
                    );
                    let (directory, path, _guard) = self.path(string(&args, "path")?, true)?;
                    let temporary = format!(".kuru-write-{}", uuid::Uuid::new_v4());
                    let mut options = OpenOptions::new();
                    options
                        .write(true)
                        .create_new(true)
                        .follow(FollowSymlinks::No);
                    let mut file = directory.open_with(&temporary, &options)?;
                    let written = (|| -> Result<()> {
                        file.write_all(content.as_bytes())?;
                        file.sync_all()?;
                        if let Ok(metadata) = directory.symlink_metadata(&path) {
                            file.set_permissions(metadata.permissions())?;
                        }
                        directory.rename(&temporary, &directory, &path)?;
                        Ok(())
                    })();
                    if written.is_err() {
                        let _ = directory.remove_file(&temporary);
                    }
                    written?;
                    Ok(ToolExecution::Text(format!(
                        "Wrote {} bytes",
                        content.len()
                    )))
                })();
                execution.map_err(ToolFailure::built_in)
            }
            "file_delete" => {
                let execution = (|| -> Result<ToolExecution> {
                    ensure!(self.allow_write, "file deletion requires allow_write=true");
                    let (directory, path, _guard) = self.path(string(&args, "path")?, true)?;
                    ensure!(
                        directory.symlink_metadata(&path)?.is_file(),
                        "file_delete requires a regular file"
                    );
                    directory.remove_file(path)?;
                    Ok(ToolExecution::Text("Deleted file".into()))
                })();
                execution.map_err(ToolFailure::built_in)
            }
            "file_list" => {
                let execution = (|| -> Result<ToolExecution> {
                    let input_path = args
                        .get("path")
                        .map(|value| value.as_str().context("path must be a string"))
                        .transpose()?
                        .unwrap_or(".");
                    let (directory, path, _guard) = self.path(input_path, false)?;
                    #[cfg(windows)]
                    let _listing_guard = Directory::open(
                        &_guard.path().join(&path),
                        Privacy::Inherited,
                        NameRetention::Pinned,
                    )?;
                    let directory = directory.open_dir_nofollow(path)?;
                    let mut entries = BTreeMap::new();
                    for entry in directory.entries()? {
                        let entry = entry?;
                        let name = entry.file_name().to_string_lossy().to_string();
                        if protected_component(&name, false) || entry.file_type()?.is_symlink() {
                            continue;
                        }
                        entries.insert(
                            name,
                            if entry.file_type()?.is_dir() {
                                "directory"
                            } else {
                                "file"
                            },
                        );
                        ensure!(
                            entries.len() <= 10_000,
                            "directory exceeds 10000 entries; select a narrower path"
                        );
                    }
                    Ok(ToolExecution::Json(serde_json::to_value(entries)?))
                })();
                execution.map_err(ToolFailure::built_in)
            }
            "shell" => {
                let execution = async {
                    ensure!(
                        self.allow_shell,
                        "shell requires allow_shell=true (process authority)"
                    );
                    let duration = args
                        .get("timeout_ms")
                        .map(|value| {
                            value
                                .as_u64()
                                .context("timeout_ms must be a positive integer")
                        })
                        .transpose()?
                        .unwrap_or(30_000);
                    ensure!(
                        (1..=120_000).contains(&duration),
                        "timeout_ms must be 1..120000"
                    );
                    #[cfg(unix)]
                    let result = shell(
                        self.shells(),
                        self.root_guard.clone(),
                        self.root.clone(),
                        string(&args, "command")?,
                        Duration::from_millis(duration),
                    )
                    .await?;
                    #[cfg(windows)]
                    let result = shell(
                        &self.root_guard,
                        &self.root,
                        string(&args, "command")?,
                        Duration::from_millis(duration),
                    )
                    .await?;
                    Ok(ToolExecution::ProjectedJson(
                        serde_json::from_str(&result).context("shell emitted invalid result")?,
                    ))
                }
                .await;
                execution.map_err(ToolFailure::built_in)
            }
            _ => match self.mcp.execute(name, args).await {
                Ok(McpExecution::Success(result)) => Ok(ToolExecution::Json(result)),
                Ok(McpExecution::ApplicationError(content)) => {
                    Ok(ToolExecution::ApplicationError {
                        kind: ToolFailureKind::McpApplication,
                        content: ToolContent::Json(content),
                    })
                }
                Err(failure) => Err(ToolFailure {
                    kind: failure.kind,
                    error: failure.error,
                }),
            },
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        #[cfg(unix)]
        {
            let (shell, mcp) = tokio::join!(self.shells.shutdown(), self.mcp.shutdown());
            match (shell, mcp) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(shell), Ok(())) => Err(shell),
                (Ok(()), Err(mcp)) => Err(mcp),
                (Err(shell), Err(mcp)) => {
                    Err(shell).context(format!("MCP shutdown also failed: {mcp:#}"))
                }
            }
        }
        #[cfg(not(unix))]
        self.mcp.shutdown().await
    }

    #[cfg(unix)]
    fn shells(&self) -> &ShellRegistry {
        &self.shells
    }

    #[cfg(all(unix, test))]
    fn test_shells(&self) -> &ShellRegistry {
        &self.shells
    }

    fn path(&self, value: &str, writing: bool) -> Result<(Dir, PathBuf, PathGuard)> {
        let path = Path::new(value);
        ensure!(
            !value.is_empty() && !path.is_absolute(),
            "tool path must be relative to the project root"
        );
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(name) => {
                    #[cfg(windows)]
                    validate_component(name)?;
                    ensure!(
                        !protected_component(&name.to_string_lossy(), writing),
                        "protected instruction, config, credential, or memory path"
                    );
                    components.push(name);
                }
                _ => bail!("tool path cannot traverse outside the project root"),
            }
        }
        let mut directory = self.directory.try_clone()?;
        let final_name = PathBuf::from(components.pop().unwrap_or(std::ffi::OsStr::new(".")));
        #[cfg(windows)]
        let guard = {
            let parent = components
                .iter()
                .fold(self.root.clone(), |path, component| path.join(component));
            let guard = Directory::open(&parent, Privacy::Inherited, NameRetention::Pinned)?;
            ensure!(
                guard.is_within(&self.root_guard)?,
                "tool parent is outside the retained project root"
            );
            self.check_expanded_names(&parent, writing)?;
            guard
        };
        #[cfg(unix)]
        let guard = ();
        for component in components {
            directory = directory.open_dir_nofollow(component)?;
        }
        match directory.symlink_metadata(&final_name) {
            Ok(metadata) => {
                ensure!(
                    !metadata.is_symlink(),
                    "symlink paths are not permitted by file tools"
                );
                #[cfg(windows)]
                self.check_expanded_names(&guard.path().join(&final_name), writing)?;
                #[cfg(windows)]
                if metadata.is_file() {
                    // All reparse types and hardlinks are checked from the native
                    // handle, beyond cap-std's ordinary symlink classification.
                    drop(guard.read(final_name.as_os_str())?);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok((directory, final_name, guard))
    }

    #[cfg(windows)]
    fn check_expanded_names(&self, path: &Path, writing: bool) -> Result<()> {
        // Confinement is already established by held native parent identities.
        // Windows canonicalization expands DOS 8.3 names; inspect that spelling
        // too so AUTH~1.JSO cannot alias an otherwise protected auth.json.
        let expanded = path.canonicalize()?;
        let relative = expanded
            .strip_prefix(&self.root)
            .context("expanded tool path is outside the project root")?;
        for component in relative.components() {
            if let Component::Normal(name) = component {
                ensure!(
                    !protected_component(&name.to_string_lossy(), writing),
                    "protected instruction, config, credential, or memory path"
                );
            }
        }
        Ok(())
    }
}

fn project_execution(execution: ToolExecution) -> Result<String> {
    match execution {
        ToolExecution::Text(text) => project_text(text),
        ToolExecution::Json(value) => project_json(value),
        ToolExecution::ProjectedText(text) => Ok(text),
        ToolExecution::ProjectedJson(value) => Ok(serde_json::to_string(&value)?),
        ToolExecution::ApplicationError { kind, content } => {
            let detail = project_content(content)?;
            Err(ProjectedToolError::new(kind, detail).into())
        }
    }
}

fn project_failure(failure: ToolFailure) -> Result<String> {
    let detail = if let Some(diagnostic) = failure
        .error
        .downcast_ref::<ProjectedShellDiagnostic>()
        .map(|diagnostic| diagnostic.0.clone())
    {
        diagnostic
    } else {
        project_text(format!("{:#}", failure.error))?
    };
    Err(ProjectedToolError::new(failure.kind, detail).into())
}

fn project_content(content: ToolContent) -> Result<String> {
    match content {
        ToolContent::Json(value) => project_json(value),
    }
}

fn project_text(text: String) -> Result<String> {
    redaction::text(&text).map_err(|_| ProjectedToolError::output_withheld().into())
}

fn project_json(value: Value) -> Result<String> {
    redaction::json(value).map_err(|_| ProjectedToolError::output_withheld().into())
}

async fn read_file_output(mut file: tokio::fs::File, extent: u64) -> Result<String> {
    let mut output = redaction::StreamingProjection::new(MAX_BYTES);
    let mut pending = Vec::new();
    let mut buffer = [0; 8192];
    let mut remaining = extent;
    while remaining > 0 {
        let read = buffer.len().min(remaining.try_into().unwrap_or(usize::MAX));
        let count = file.read(&mut buffer[..read]).await?;
        if count == 0 {
            break;
        }
        remaining -= count as u64;
        pending.extend_from_slice(&buffer[..count]);
        match std::str::from_utf8(&pending) {
            Ok(_) => {
                output.push(&pending)?;
                pending.clear();
            }
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                output.push(&pending[..valid])?;
                pending = pending[valid..].to_vec();
            }
            Err(_) => bail!("file is not UTF-8"),
        }
    }
    ensure!(pending.is_empty(), "file is not UTF-8");
    output.finish().map_err(Into::into)
}

fn protected_component(value: &str, writing: bool) -> bool {
    let name = value.to_ascii_lowercase();
    matches!(
        name.as_str(),
        ".git"
            | ".kuru"
            | ".codex"
            | ".agents"
            | ".claude"
            | ".ssh"
            | ".aws"
            | ".azure"
            | ".gnupg"
            | ".mcp.json"
            | "auth.json"
            | "credentials.json"
            | "memory.sqlite"
            | "memory.db"
    ) || name == ".env"
        || name.starts_with(".env.")
        || (writing
            && matches!(
                name.as_str(),
                "agents.md" | "claude.md" | "mise.toml" | "hk.pkl"
            ))
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("{key} must be a string"))
}

fn spec(name: &str, description: &str, fields: &[&str], required: &[&str]) -> ToolSpec {
    let properties: BTreeMap<_, _> = fields
        .iter()
        .map(|name| (*name, json!({"type":"string"})))
        .collect();
    ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}

#[cfg(unix)]
async fn shell(
    registry: &ShellRegistry,
    root_guard: Arc<Directory>,
    root: PathBuf,
    command: &str,
    duration: Duration,
) -> Result<String> {
    registry
        .execute(root_guard, root, command.into(), duration, || {
            unix_shell_environment(std::env::vars_os())
        })
        .await
}

#[cfg(windows)]
async fn shell(
    root_guard: &Directory,
    root: &Path,
    command: &str,
    duration: Duration,
) -> Result<String> {
    use base64::Engine;
    use kuru_platform::windows::process::{
        Stdio, configured_command, system_directory, wait_process_handle,
    };
    ensure!(!command.trim().is_empty(), "shell command is empty");
    // This is deliberately authorized PowerShell source, not command argv.
    // Stock Windows PowerShell accepts UTF-16LE source; its actual exit status
    // is returned without a suffix that could accidentally replace `$?`.
    // Headless progress (including first-use module discovery) is not a text
    // diagnostic: Windows PowerShell 5.1 serializes it onto redirected stderr.
    // Disable only progress before any cmdlet runs; preserve warning/error and
    // literal stderr bytes, including text which happens to resemble CLIXML.
    let source = format!(
        "$ProgressPreference = 'SilentlyContinue'; [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding;\n{command}"
    );
    let bytes: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut args: Vec<std::ffi::OsString> = [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-OutputFormat",
        "Text",
        "-EncodedCommand",
    ]
    .map(Into::into)
    .into();
    args.push(
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into(),
    );
    // Use the same identity-checked launch spelling as other configured native
    // commands. PowerShell's .NET file APIs cannot use an introduced verbatim cwd.
    let system_directory = system_directory()?;
    let program = system_directory.join("WindowsPowerShell/v1.0/powershell.exe");
    // This owned stock-shell launch must reconstruct its own module paths: a
    // PowerShell 7 parent can otherwise leave incompatible modules through Kuru.
    // Generic configured commands retain their caller's deliberate environment.
    let environment = windows_shell_environment(std::env::vars_os(), &system_directory)?;
    let mut spec = configured_command(program.as_os_str(), &args, root, environment)?;
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    root_guard.revalidate()?;
    let mut child = spec
        .spawn()
        .await
        .context("cannot start Windows PowerShell")?;
    let mut stdout = child.take_stdout().context("missing shell stdout")?;
    let mut stderr = child.take_stderr().context("missing shell stderr")?;
    let mut out = ShellCapture::new();
    let mut err = ShellCapture::new();
    let mut phase = "read shell output";
    let operation = async {
        tokio::try_join!(out.read(&mut stdout), err.read(&mut stderr))?;
        out.finish()?;
        err.finish()?;
        phase = "wait for shell process tree";
        let status = child.wait(duration).await?;
        Ok::<_, anyhow::Error>(status)
    };
    let result = timeout(duration, operation)
        .await
        .context("shell timed out")
        .and_then(|result| result);
    let mut failure_metadata = None;
    let result = match result {
        Ok(status) => Ok(status),
        Err(error) => {
            #[cfg(all(test, windows))]
            let mut stack_diagnostic = if tests::windows_stack_diagnostic_selected() {
                Some(tests::capture_selected_windows_stack(&mut child, root).await)
            } else {
                None
            };
            // Query the retained root separately from whole-Job quiescence;
            // observation failure must never prevent the existing cleanup.
            let root_state = match child.duplicate_process_handle() {
                Ok(process) => match wait_process_handle(&process, Duration::ZERO).await {
                    Ok(()) => "exited",
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => "running",
                    Err(_) => "query-error",
                },
                Err(_) => "query-error",
            };
            let observed = child.try_wait();
            let stopped_result = crate::process::stop(&mut child).await;
            let stopped = match &stopped_result {
                Ok(()) => "subprocess tree terminated".to_owned(),
                Err(error) => format!("subprocess cleanup unconfirmed: {error:#}"),
            };
            #[cfg(all(test, windows))]
            let stopped = {
                let mut stopped = stopped;
                if let Some(diagnostic) = &mut stack_diagnostic {
                    diagnostic.finish_after_target(stopped_result.is_ok());
                    if !diagnostic.cleanup_confirmed {
                        stopped.push_str("; CDB cleanup unconfirmed");
                    }
                }
                stopped
            };
            failure_metadata = Some(ShellFailureMetadata {
                phase: phase.to_owned(),
                root_state: root_state.to_owned(),
                observed: format!("{observed:?}"),
                stopped,
                #[cfg(all(test, windows))]
                stack_diagnostic: stack_diagnostic.map(|diagnostic| diagnostic.detail),
            });
            Err(error)
        }
    };
    let cleanup = async {
        let (out, err) = tokio::join!(
            stdout.close(Duration::from_secs(5)),
            stderr.close(Duration::from_secs(5)),
        );
        out?;
        err?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    match result {
        Ok(status) => {
            cleanup?;
            Ok(json!({"exit_code":status.code(),"success":status.success(),"stdout":out.text,"stderr":err.text}).to_string())
        }
        Err(error) => {
            let error = match cleanup {
                Ok(()) => error,
                Err(cleanup) => error.context(format!("shell pipe cleanup failed: {cleanup:#}")),
            };
            Err(projected_shell_failure(
                error,
                &failure_metadata.expect("shell failure metadata was recorded"),
                &out,
                &err,
            )?)
        }
    }
}

/// A source-free shell failure that has already combined and projected all raw
/// operation, cleanup, and process-observation details.
#[derive(Debug)]
struct ProjectedShellDiagnostic(String);

impl std::fmt::Display for ProjectedShellDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProjectedShellDiagnostic {}

#[cfg(any(windows, test))]
struct ShellFailureMetadata {
    phase: String,
    root_state: String,
    observed: String,
    stopped: String,
    #[cfg(test)]
    stack_diagnostic: Option<String>,
}

#[cfg(any(windows, test))]
fn projected_shell_failure(
    error: anyhow::Error,
    metadata: &ShellFailureMetadata,
    stdout: &ShellCapture,
    stderr: &ShellCapture,
) -> Result<anyhow::Error> {
    let raw = format!(
        "{error:#}; {}; stdout {} bytes (EOF {}); stderr {} bytes (EOF {}); root before cleanup: {}; tree before cleanup: {}; {}",
        metadata.phase,
        stdout.observed,
        stdout.eof,
        stderr.observed,
        stderr.eof,
        metadata.root_state,
        metadata.observed,
        metadata.stopped,
    );
    #[cfg(test)]
    let raw = match &metadata.stack_diagnostic {
        Some(diagnostic) => format!("{raw}; CDB stack diagnostic: {diagnostic}"),
        None => raw,
    };
    let diagnostic = format!(
        "{}; stderr prefix: {}",
        project_text(raw)?,
        shell_stderr_diagnostic(stderr),
    );
    Ok(ProjectedShellDiagnostic(diagnostic).into())
}

// Keep accepted output outside the cancellable read future. A timeout must not
// discard the bytes and EOF observations needed to distinguish a running shell
// from a completed process whose output is still held by another process.
#[cfg(any(windows, test))]
struct ShellCapture {
    projection: Option<redaction::StreamingProjection>,
    text: Option<String>,
    observed: usize,
    eof: bool,
    #[cfg(test)]
    bytes: Vec<u8>,
}

#[cfg(any(windows, test))]
fn shell_stderr_diagnostic(capture: &ShellCapture) -> String {
    const DIAGNOSTIC_BYTES: usize = 4096;

    if capture.eof {
        capture
            .text
            .as_deref()
            .map(|text| redaction::truncate_tool_output(text, DIAGNOSTIC_BYTES))
            .unwrap_or_default()
    } else {
        "<pending EOF>".to_owned()
    }
}

#[cfg(any(windows, test))]
impl ShellCapture {
    fn new() -> Self {
        Self {
            projection: Some(redaction::StreamingProjection::new(MAX_BYTES)),
            text: None,
            observed: 0,
            eof: false,
            #[cfg(test)]
            bytes: Vec::new(),
        }
    }

    async fn read(&mut self, reader: &mut (impl tokio::io::AsyncRead + Unpin)) -> Result<()> {
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer).await?;
            if count == 0 {
                self.eof = true;
                return Ok(());
            }
            self.observed = self.observed.saturating_add(count);
            self.projection
                .as_mut()
                .expect("shell capture was already finished")
                .push(&buffer[..count])?;
            #[cfg(test)]
            self.bytes.extend_from_slice(&buffer[..count]);
        }
    }

    fn finish(&mut self) -> Result<()> {
        self.text = Some(
            self.projection
                .take()
                .expect("shell capture was already finished")
                .finish()?,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{HttpFixture, Reply, drain_bounded};
    #[cfg(unix)]
    use crate::test_support::{StdioFixture, Step};
    use kuru_core::McpConfig;

    #[tokio::test]
    async fn cancelled_shell_capture_preserves_received_bytes_until_actual_eof() {
        use tokio::io::AsyncWriteExt;
        let (mut reader, mut writer) = tokio::io::duplex(1);
        let (sent, received) = tokio::sync::oneshot::channel();
        let (finish, finishing) = tokio::sync::oneshot::channel();
        let producer = tokio::spawn(async move {
            // The one-byte pipe cannot accept the suffix until the whole prefix
            // has been read. Then retain the writer to withhold real EOF.
            writer.write_all(b"accepted output!").await.unwrap();
            sent.send(()).unwrap();
            finishing.await.unwrap();
            writer.shutdown().await.unwrap();
        });
        let mut capture = ShellCapture::new();
        timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = capture.read(&mut reader) => panic!("writer still open: {result:?}"),
                result = received => result.unwrap(),
            }
        })
        .await
        .unwrap();
        assert!(capture.bytes.starts_with(b"accepted output"));
        assert!(!capture.eof);
        finish.send(()).unwrap();
        timeout(Duration::from_secs(2), capture.read(&mut reader))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(capture.bytes, b"accepted output!");
        assert!(capture.eof);
        capture.finish().unwrap();
        assert_eq!(capture.text.as_deref(), Some("accepted output!"));
        producer.await.unwrap();
    }

    #[tokio::test]
    async fn shell_capture_retains_marked_head_and_tail_after_overflow() {
        let bytes = vec![b'x'; MAX_BYTES + 16384];
        let mut source = bytes.as_slice();
        let mut capture = ShellCapture::new();
        capture.read(&mut source).await.unwrap();
        capture.finish().unwrap();
        let text = capture.text.as_deref().unwrap();
        assert!(text.len() <= MAX_BYTES);
        assert!(text.starts_with('x'));
        assert!(text.ends_with('x'));
        assert!(text.contains("[truncated]"));
        assert_eq!(capture.observed, bytes.len());
        assert!(capture.eof);
    }

    #[tokio::test]
    async fn shell_capture_lossily_preserves_bounded_invalid_utf8_head_and_tail() {
        let mut bytes = b"HEAD\xff".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', MAX_BYTES));
        bytes.extend(b"\xfe:TAIL");
        let mut source = bytes.as_slice();
        let mut capture = ShellCapture::new();
        capture.read(&mut source).await.unwrap();
        capture.finish().unwrap();
        let text = capture.text.as_deref().unwrap();
        assert!(text.len() <= MAX_BYTES);
        assert!(text.starts_with("HEAD?"));
        assert!(text.ends_with("?:TAIL"));
        assert!(text.contains("[truncated]"));
        assert!(capture.eof);
    }

    #[tokio::test]
    async fn shell_error_diagnostic_bounds_projected_stderr_without_reprojecting_marker() {
        let secret = "sk-proj-abcdefghijklmnop0123456789";
        let bytes = format!("openai_api_key={secret}\n{}:TAIL", "x".repeat(16 * 1024));
        let mut source = bytes.as_bytes();
        let mut capture = ShellCapture::new();
        capture.read(&mut source).await.unwrap();
        capture.finish().unwrap();

        let diagnostic = shell_stderr_diagnostic(&capture);
        assert!(diagnostic.len() <= 4096);
        assert!(diagnostic.contains("[REDACTED:recognized-secret]"));
        assert!(diagnostic.contains("[truncated]"));
        assert!(diagnostic.ends_with(":TAIL"));
        assert!(!diagnostic.contains(secret));
        let without_markers = diagnostic
            .replace("[REDACTED:recognized-secret]", "")
            .replace("[truncated]", "");
        assert!(!without_markers.contains(['[', ']']));

        let operation_secret = "sk-proj-operation-secret-0123456789";
        let cleanup_secret = "sk-proj-cleanup-secret-0123456789";
        let metadata_secret = "sk-proj-metadata-secret-0123456789";
        let error = anyhow::anyhow!("shell process failure {operation_secret}")
            .context(format!("shell pipe cleanup failed: {cleanup_secret}"));
        let metadata = ShellFailureMetadata {
            phase: format!("read shell output {metadata_secret}"),
            root_state: format!("query-error {metadata_secret}"),
            observed: format!("Err({metadata_secret})"),
            stopped: format!(
                "subprocess cleanup unconfirmed: {metadata_secret}; CDB cleanup unconfirmed"
            ),
            stack_diagnostic: Some(format!("native stack: {metadata_secret}")),
        };
        let error = projected_shell_failure(error, &metadata, &capture, &capture).unwrap();
        assert!(error.downcast_ref::<ProjectedShellDiagnostic>().is_some());
        assert_eq!(error.chain().count(), 1);
        let rendered = project_failure(ToolFailure::built_in(error))
            .unwrap_err()
            .to_string();
        assert!(rendered.contains("[REDACTED:recognized-secret]"));
        assert!(rendered.contains("[truncated]"));
        assert!(rendered.contains(":TAIL"));
        assert!(rendered.contains("CDB cleanup unconfirmed"));
        assert!(rendered.contains("CDB stack diagnostic"));
        for secret in [secret, operation_secret, cleanup_secret, metadata_secret] {
            assert!(!rendered.contains(secret));
        }
        let without_markers = rendered
            .replace("[REDACTED:recognized-secret]", "")
            .replace("[truncated]", "");
        assert!(
            !without_markers.contains(['[', ']']),
            "partial marker in {rendered:?}"
        );
    }

    #[tokio::test]
    async fn file_crud_is_contained_and_replaces_atomically() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("AGENTS.md"), "instructions").unwrap();
        std::fs::write(root.path().join(".env"), "protected").unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            host.specs()
                .await
                .unwrap()
                .iter()
                .any(|tool| tool.name == "file_write")
        );
        host.execute("file_write", json!({"path":"src/a.txt","content":"one"}))
            .await
            .unwrap();
        host.execute("file_write", json!({"path":"src/a.txt","content":"two"}))
            .await
            .unwrap();
        assert_eq!(
            host.execute("file_read", json!({"path":"./src/a.txt"}))
                .await
                .unwrap(),
            "two"
        );
        let entries: Value =
            serde_json::from_str(&host.execute("file_list", json!({})).await.unwrap()).unwrap();
        assert_eq!(entries["src"], "directory");
        assert!(entries.get(".env").is_none());
        assert_eq!(
            host.execute("file_read", json!({"path":"AGENTS.md"}))
                .await
                .unwrap(),
            "instructions"
        );
        for protected in [
            "../outside",
            "/etc/passwd",
            ".kuru/memory.sqlite",
            ".env",
            "src/../a",
            "",
        ] {
            assert!(
                host.execute("file_read", json!({"path":protected}))
                    .await
                    .is_err(),
                "{protected}"
            );
        }
        for protected in [
            "AGENTS.md",
            "CLAUDE.md",
            "mise.toml",
            "hk.pkl",
            ".mcp.json",
            ".codex/config.toml",
        ] {
            assert!(
                host.execute(
                    "file_write",
                    json!({"path":protected,"content":"overwritten"})
                )
                .await
                .is_err(),
                "{protected}"
            );
        }
        assert!(
            host.execute("file_write", json!({"path":"missing/child","content":"x"}))
                .await
                .is_err()
        );
        assert!(
            host.execute("file_write", json!({"path":"src","content":"x"}))
                .await
                .is_err()
        );
        assert!(
            host.execute("file_delete", json!({"path":"src"}))
                .await
                .is_err()
        );
        host.execute("file_delete", json!({"path":"src/a.txt"}))
            .await
            .unwrap();
        assert!(!root.path().join("src/a.txt").exists());
        assert!(
            host.execute("file_read", json!({"path":"src"}))
                .await
                .is_err()
        );
        assert!(host.execute("file_read", json!({"path":1})).await.is_err());
        assert!(host.execute("file_read", json!([])).await.is_err());
        assert!(host.execute("unknown", json!({})).await.is_err());
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn tool_projection_redacts_results_without_altering_files_or_arguments() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        const MARKER: &str = "[REDACTED:recognized-secret]";
        let root = tempfile::tempdir().unwrap();
        let filename = "read.txt";
        let source = format!("ordinary {SECRET} retained on disk");
        std::fs::write(root.path().join(filename), &source).unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();

        let read = host
            .execute("file_read", json!({"path":&filename}))
            .await
            .unwrap();
        assert!(read.contains(MARKER));
        assert!(!read.contains(SECRET));
        assert_eq!(
            std::fs::read_to_string(root.path().join(filename)).unwrap(),
            source
        );

        let written = "write.txt";
        host.execute("file_write", json!({"path":&written,"content":&source}))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join(written)).unwrap(),
            source
        );

        let listing = host.execute("file_list", json!({})).await.unwrap();
        let listing: Value = serde_json::from_str(&listing).unwrap();
        assert_eq!(listing[filename], "file");
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn file_read_accepts_exactly_the_existing_byte_limit() {
        let root = tempfile::tempdir().unwrap();
        let contents = "x".repeat(MAX_BYTES);
        std::fs::write(root.path().join("at-limit"), &contents).unwrap();
        let host = ToolHost::new(root.path(), &Config::default()).unwrap();
        assert_eq!(
            host.execute("file_read", json!({"path":"at-limit"}))
                .await
                .unwrap(),
            contents
        );
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_utf8_file_failure_withholds_original_bytes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("bad"), b"sk-abcdefghijklmnop\xff").unwrap();
        let host = ToolHost::new(root.path(), &Config::default()).unwrap();
        let error = host
            .execute("file_read", json!({"path":"bad"}))
            .await
            .unwrap_err();
        for rendered in [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
        ] {
            assert!(rendered.contains("file is not UTF-8"));
            assert!(!rendered.contains("sk-abcdefghijklmnop"));
        }
        assert_eq!(error.chain().count(), 1);
        assert!(error.downcast_ref::<std::string::FromUtf8Error>().is_none());
        host.shutdown().await.unwrap();
    }

    #[test]
    fn projected_tool_failures_do_not_retain_raw_error_sources() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let error =
            anyhow::anyhow!("outer failure {SECRET}").context(format!("inner failure {SECRET}"));
        let error = project_failure(ToolFailure::built_in(error)).unwrap_err();
        for rendered in [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
            error
                .chain()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | "),
        ] {
            assert!(rendered.contains("[REDACTED:recognized-secret]"));
            assert!(!rendered.contains(SECRET));
        }
        assert_eq!(error.chain().count(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn toolhost_stdio_mcp_projects_application_success_and_protocol_results() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let peer = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"remote","inputSchema":{"type":"object"}}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":3,"result":{"isError":true,"content":[{"type":"text","text":format!("denied {SECRET}")}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":format!("usable {SECRET}")}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":5,"error":{"code":-32000,"message":format!("failed {SECRET}")}}),
            ),
            Step::Eof,
        ]);
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "fixture".into(),
                    McpConfig {
                        command: Some(peer.command().into()),
                        args: vec![],
                        url: None,
                        env: BTreeMap::new(),
                    },
                )]
                .into(),
                ..Default::default()
            },
        )
        .unwrap();
        let name = host
            .specs()
            .await
            .unwrap()
            .into_iter()
            .find(|spec| spec.description.starts_with("MCP fixture/remote:"))
            .expect("missing discovered fixture MCP tool")
            .name;
        let application = host.execute(&name, json!({})).await.unwrap_err();
        for rendered in [
            application.to_string(),
            format!("{application:#}"),
            format!("{application:?}"),
        ] {
            assert!(rendered.contains("denied"));
            assert!(rendered.contains("[REDACTED:recognized-secret]"));
            assert!(!rendered.contains(SECRET));
        }
        let success = host.execute(&name, json!({})).await.unwrap();
        assert!(success.contains("[REDACTED:recognized-secret]"));
        assert!(!success.contains(SECRET));
        let failure = host.execute(&name, json!({})).await.unwrap_err();
        assert_eq!(
            failure.to_string(),
            "MCP tool call failed: configured MCP server fixture is unavailable"
        );
        assert!(!format!("{failure:#} {failure:?}").contains(SECRET));
        assert_eq!(failure.chain().count(), 1);
        host.shutdown().await.unwrap();
        assert_eq!(
            peer.conversations(),
            vec![vec![
                json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "method":"initialize",
                    "params":{
                        "protocolVersion":"2025-11-25",
                        "capabilities":{},
                        "clientInfo":{"name":"kuru","version":env!("CARGO_PKG_VERSION")},
                    },
                }),
                json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"remote","arguments":{}}}),
                json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"remote","arguments":{}}}),
                json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"remote","arguments":{}}}),
            ]],
            "the protocol-error cleanup may terminate the peer before its planned EOF, but it must not skip or replay a request"
        );
    }

    #[tokio::test]
    async fn toolhost_http_mcp_projects_application_success_and_protocol_results() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let mut initialized =
            Reply::rpc(json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}));
        initialized.session = true;
        let peer = HttpFixture::new(vec![
            initialized,
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[{"name":"remote","description":"fixture","inputSchema":{"type":"object"}}]})),
            Reply::rpc(json!({"isError":true,"content":[{"type":"text","text":format!("denied {SECRET}")}]})),
            Reply::rpc(json!({"content":[{"type":"text","text":format!("usable {SECRET}")}],"structuredContent":{"nested":{"value":7}}})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":format!("failed {SECRET}")}})),
            Reply::json(json!({})),
        ]).await;
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "http".into(),
                    McpConfig {
                        command: None,
                        args: vec![],
                        url: Some(peer.url.clone()),
                        env: BTreeMap::new(),
                    },
                )]
                .into(),
                ..Default::default()
            },
        )
        .unwrap();
        let name = host
            .specs()
            .await
            .unwrap()
            .into_iter()
            .find(|spec| spec.description.starts_with("MCP http/remote:"))
            .expect("missing discovered HTTP MCP tool")
            .name;
        let application = host
            .execute(&name, json!({"input": SECRET}))
            .await
            .unwrap_err();
        for rendered in [
            application.to_string(),
            format!("{application:#}"),
            format!("{application:?}"),
        ] {
            assert!(rendered.contains("denied"));
            assert!(rendered.contains("[REDACTED:recognized-secret]"));
            assert!(!rendered.contains(SECRET));
        }
        let success: Value =
            serde_json::from_str(&host.execute(&name, json!({})).await.unwrap()).unwrap();
        assert_eq!(success["structuredContent"]["nested"]["value"], 7);
        assert_eq!(
            success["content"][0]["text"],
            "usable [REDACTED:recognized-secret]"
        );
        let failure = host.execute(&name, json!({})).await.unwrap_err();
        for rendered in [
            failure.to_string(),
            format!("{failure:#}"),
            format!("{failure:?}"),
        ] {
            assert!(rendered.contains("configured MCP server http is unavailable"));
            assert!(!rendered.contains(SECRET));
        }
        assert_eq!(failure.chain().count(), 1);
        host.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 7);
        assert_eq!(requests[3].body["method"], "tools/call");
        assert_eq!(
            requests[3].body["params"]["arguments"],
            json!({"input": SECRET})
        );
    }

    #[tokio::test]
    async fn file_list_keeps_its_independent_entry_cap_above_result_limit() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..10_000 {
            let name = format!("{index:05}-{}", "x".repeat(238));
            std::fs::write(root.path().join(name), []).unwrap();
        }
        let host = ToolHost::new(root.path(), &Config::default()).unwrap();
        let listed = host.execute("file_list", json!({})).await.unwrap();
        assert!(listed.len() > MAX_BYTES);
        let listed: Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed.as_object().unwrap().len(), 10_000);
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn toolhost_http_mcp_accepts_an_exact_wire_body_limit() {
        let mut initialized =
            Reply::rpc(json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}));
        initialized.session = true;
        let empty = Reply::rpc(json!({"content":[{"type":"text","text":""}]}));
        let payload = "x".repeat(MAX_BYTES - empty.body.replace("\"$ID\"", "3").len());
        let reply = Reply::rpc(json!({"content":[{"type":"text","text":payload}]}));
        assert_eq!(reply.body.replace("\"$ID\"", "3").len(), MAX_BYTES);
        let peer = HttpFixture::new(vec![initialized, Reply::json(json!({})), Reply::rpc(json!({"tools":[{"name":"remote","description":"fixture","inputSchema":{"type":"object"}}]})), reply, Reply::json(json!({}))]).await;
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "http".into(),
                    McpConfig {
                        command: None,
                        args: vec![],
                        url: Some(peer.url.clone()),
                        env: BTreeMap::new(),
                    },
                )]
                .into(),
                ..Default::default()
            },
        )
        .unwrap();
        let name = host
            .specs()
            .await
            .unwrap()
            .into_iter()
            .find(|spec| spec.description.starts_with("MCP http/remote:"))
            .unwrap()
            .name;
        let value: Value =
            serde_json::from_str(&host.execute(&name, json!({})).await.unwrap()).unwrap();
        host.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests[3].body["id"], 3);
        assert_eq!(value, json!({"content":[{"type":"text","text":payload}]}));
    }

    #[tokio::test]
    async fn permissions_and_file_reads_preserve_bounded_visible_output() {
        let root = tempfile::tempdir().unwrap();
        let read_only = ToolHost::new(root.path(), &Config::default()).unwrap();
        let names: Vec<_> = read_only
            .specs()
            .await
            .unwrap()
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(names, vec!["file_read", "file_list"]);
        for name in ["file_write", "file_delete", "shell"] {
            assert!(
                read_only
                    .execute(name, json!({"path":"x","content":"x","command":"true"}))
                    .await
                    .is_err()
            );
        }
        std::fs::write(
            root.path().join("large"),
            format!("HEAD:{}:TAIL", "x".repeat(MAX_BYTES)),
        )
        .unwrap();
        std::fs::write(root.path().join("binary"), [0xff]).unwrap();
        let large = read_only
            .execute("file_read", json!({"path":"large"}))
            .await
            .unwrap();
        assert!(large.len() <= MAX_BYTES);
        assert!(large.starts_with("HEAD:"));
        assert!(large.ends_with(":TAIL"));
        assert!(large.contains("[truncated]"));
        assert!(
            read_only
                .execute("file_read", json!({"path":"binary"}))
                .await
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        let write = ToolHost::new(
            root.path(),
            &Config {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            write
                .execute(
                    "file_write",
                    json!({"path":"x","content":"x".repeat(MAX_BYTES+1)})
                )
                .await
                .is_err()
        );
        assert!(!root.path().join("x").exists());
        assert!(ToolHost::new(&root.path().join("does-not-exist"), &Config::default()).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn links_cannot_escape_or_alias_protected_files() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "outside").unwrap();
        std::fs::create_dir(root.path().join(".kuru")).unwrap();
        std::fs::write(root.path().join(".kuru/memory"), "private").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        symlink(root.path().join(".kuru/memory"), root.path().join("alias")).unwrap();
        std::fs::hard_link(outside.path().join("secret"), root.path().join("hardlink")).unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();
        for path in ["escape/secret", "alias", "hardlink"] {
            assert!(
                host.execute("file_read", json!({"path":path}))
                    .await
                    .is_err()
            );
        }
        for path in ["escape/secret", "alias"] {
            assert!(
                host.execute("file_write", json!({"path":path,"content":"bad"}))
                    .await
                    .is_err()
            );
        }
        // Atomic replacement disconnects a hard link; it cannot alter its target.
        host.execute("file_write", json!({"path":"hardlink","content":"local"}))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret")).unwrap(),
            "outside"
        );
        let listed = host.execute("file_list", json!({})).await.unwrap();
        assert!(!listed.contains("escape") && !listed.contains("alias"));
    }

    #[tokio::test]
    async fn shell_returns_status_bounds_output_and_terminates_on_timeout() {
        #[cfg(unix)]
        let (command, stall, flood, stderr_flood) = (
            "printf hello; printf problem >&2; exit 7",
            "sleep 5",
            "head -c 2097153 /dev/zero | tr '\\0' x",
            "head -c 2097153 /dev/zero | tr '\\0' x >&2",
        );
        #[cfg(windows)]
        let (command, stall, flood, stderr_flood) = (
            "[Console]::Out.Write('hello'); [Console]::Error.Write('problem'); exit 7",
            "Start-Sleep -Seconds 5",
            "[Console]::Out.Write('x' * 2097153)",
            "[Console]::Error.Write('x' * 2097153)",
        );
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            host.specs()
                .await
                .unwrap()
                .iter()
                .any(|spec| spec.name == "shell")
        );
        let output: Value = serde_json::from_str(
            &host
                .execute("shell", json!({"command":command}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(output["stdout"], "hello");
        assert_eq!(output["stderr"], "problem");
        assert_eq!(output["exit_code"], 7);
        assert_eq!(output["success"], false);
        assert!(
            host.execute("shell", json!({"command":stall,"timeout_ms":20}))
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        for invalid in [json!(0), json!(120001), json!("1")] {
            assert!(
                host.execute("shell", json!({"command":"true","timeout_ms":invalid}))
                    .await
                    .is_err()
            );
        }
        assert!(host.execute("shell", json!({"command":""})).await.is_err());
        for (command, stream, head, tail) in [
            (flood, "stdout", "x", "x"),
            (stderr_flood, "stderr", "x", "x"),
        ] {
            let output: Value = serde_json::from_str(
                &host
                    .execute("shell", json!({"command":command}))
                    .await
                    .unwrap(),
            )
            .unwrap();
            let retained = output[stream].as_str().unwrap();
            assert!(output["success"] == true);
            assert!(retained.len() <= MAX_BYTES);
            assert!(retained.starts_with(head));
            assert!(retained.ends_with(tail));
            assert!(retained.contains("[truncated]"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_keeps_independent_near_limit_stdout_and_stderr() {
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let output = host
            .execute(
                "shell",
                json!({
                    "command": "head -c 2097152 /dev/zero | tr '\\0' o; head -c 2097152 /dev/zero | tr '\\0' e >&2"
                }),
            )
            .await
            .unwrap();
        let output: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(output["stdout"], "o".repeat(MAX_BYTES));
        assert_eq!(output["stderr"], "e".repeat(MAX_BYTES));
        assert!(output["success"] == true);
        host.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_projection_redacts_both_streams_without_changing_exit_status() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let output = host
            .execute(
                "shell",
                json!({"command":format!("printf 'out {SECRET}'; printf 'err {SECRET}' >&2; exit 7")}),
            )
            .await
            .unwrap();
        let output: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(output["stdout"], "out [REDACTED:recognized-secret]");
        assert_eq!(output["stderr"], "err [REDACTED:recognized-secret]");
        assert_eq!(output["exit_code"], 7);
        assert!(output["success"] == false);
        host.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shell_timeout_projects_a_fixed_failure_without_captured_stderr() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("timeout-ready");
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let command = format!("printf '%s' '{SECRET}' >&2; : > timeout-ready; exec sleep 5");
        let (ready_result, call_result) = tokio::join!(
            timeout(Duration::from_secs(5), async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }),
            timeout(
                Duration::from_secs(10),
                host.execute(
                    "shell",
                    json!({
                        "command": command,
                        "timeout_ms": 3_000,
                    }),
                ),
            )
        );
        let shutdown_result = timeout(Duration::from_secs(6), host.shutdown()).await;

        assert!(
            matches!(shutdown_result, Ok(Ok(()))),
            "timed shell shutdown did not finish"
        );
        ready_result.expect("timed shell did not reach readiness before the bounded wait");
        let error = call_result
            .expect("timed shell did not return within its timeout and cleanup allowance")
            .unwrap_err();
        let rendered = [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
            error
                .chain()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | "),
        ];
        let chain_length = error.chain().count();

        assert_eq!(rendered[0], "tool execution failed: shell timed out");
        for output in rendered {
            assert!(!output.contains(SECRET));
            assert!(!output.contains(&command));
            assert!(!output.contains("[REDACTED:recognized-secret]"));
        }
        assert_eq!(chain_length, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shell_requires_both_eof_and_root_exit_then_reaps_descendants() {
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let started = std::time::Instant::now();
        let output: Value = serde_json::from_str(
            &host
                .execute("shell", json!({"command":"exec 1>&- 2>&-; sleep 0.15"}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert_eq!(output["exit_code"], 0);
        assert_eq!(output["stdout"], "");
        assert_eq!(output["stderr"], "");

        let output: Value = serde_json::from_str(
            &host
                .execute(
                    "shell",
                    json!({"command":"sleep 5 </dev/null >/dev/null 2>/dev/null & exit 7"}),
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(output["exit_code"], 7);
        assert_eq!(output["success"], false);
        host.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shell_caller_loss_keeps_registered_owner_for_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("caller-ready");
        let host = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    allow_shell: true,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let call = tokio::spawn({
            let host = host.clone();
            async move {
                host.execute(
                    "shell",
                    json!({"command":": > caller-ready; exec sleep 5", "timeout_ms":120_000}),
                )
                .await
            }
        });
        timeout(Duration::from_secs(2), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("shell caller did not reach readiness");
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        timeout(Duration::from_secs(6), host.shutdown())
            .await
            .expect("registered shell owner did not finish bounded shutdown")
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shutdown_cancels_starting_and_active_shells_and_closes_mcp() {
        let peer = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}})),
            Step::Eof,
        ]);
        let root = tempfile::tempdir().unwrap();
        let active_ready = root.path().join("active-ready");
        let host = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    allow_shell: true,
                    mcp: [(
                        "fixture".into(),
                        McpConfig {
                            command: Some(peer.command().into()),
                            args: vec![],
                            url: None,
                            env: BTreeMap::new(),
                        },
                    )]
                    .into(),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        assert!(
            host.specs()
                .await
                .unwrap()
                .iter()
                .any(|spec| spec.name == "shell")
        );
        let starting_gate = host.test_shells().test_arm_start_gate();
        let starting = tokio::spawn({
            let host = host.clone();
            async move {
                host.execute(
                    "shell",
                    json!({"command":": > started-after-shutdown", "timeout_ms":120_000}),
                )
                .await
            }
        });
        timeout(Duration::from_secs(1), async {
            while host.test_shells().test_owner_count() != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("starting shell was not registered");
        timeout(Duration::from_secs(1), async {
            while !starting_gate.entered() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("starting shell did not reach its controlled gate");
        let active = tokio::spawn({
            let host = host.clone();
            async move {
                host.execute(
                    "shell",
                    json!({"command":": > active-ready; exec sleep 5", "timeout_ms":120_000}),
                )
                .await
            }
        });
        let active_ready_result = timeout(Duration::from_secs(2), async {
            while !active_ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if active_ready_result.is_err() {
            let active_result = timeout(Duration::from_millis(100), active).await;
            starting_gate.release();
            let _ = host.shutdown().await;
            panic!("active shell did not reach readiness: {active_result:?}");
        }
        let shutdown = tokio::spawn({
            let host = host.clone();
            async move { host.shutdown().await }
        });
        timeout(Duration::from_secs(1), async {
            while !host.test_shells().test_is_closing() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("shutdown did not close shell registration");
        let rejected = host
            .execute("shell", json!({"command":": > launched-after-shutdown"}))
            .await
            .unwrap_err();
        assert!(rejected.to_string().contains("shutting down"));
        starting_gate.release();
        timeout(Duration::from_secs(6), shutdown)
            .await
            .expect("combined shell/MCP shutdown did not finish")
            .unwrap()
            .unwrap();
        assert!(
            starting
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        assert!(
            active
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        peer.assert_completed(1);
        assert!(!root.path().join("started-after-shutdown").exists());
        assert!(!root.path().join("launched-after-shutdown").exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_outlives_a_destroyed_parent_runtime() {
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("worker-ready");
        let host = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    allow_shell: true,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let root = root.path().to_path_buf();
            runtime.block_on(async {
                let _call = tokio::spawn({
                    let host = host.clone();
                    async move {
                        host.execute(
                            "shell",
                            json!({"command":": > worker-ready; exec sleep 5", "timeout_ms":120_000}),
                        )
                        .await
                    }
                });
                timeout(Duration::from_secs(2), async {
                    while !root.join("worker-ready").exists() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("shell worker did not reach readiness before runtime destruction");
            });
        }
        assert!(ready.exists());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            timeout(Duration::from_secs(6), host.shutdown())
                .await
                .expect("retained worker did not finish after parent runtime destruction")
                .unwrap();
        });
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retained_root_replacement_blocks_shell_and_keeps_file_capability() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("held.txt"), "held object").unwrap();
        let retained =
            Arc::new(Directory::open(&root, Privacy::Inherited, NameRetention::Pinned).unwrap());
        let host = ToolHost::with_retained_root(
            retained,
            &Config {
                allow_shell: true,
                ..Config::default()
            },
        )
        .unwrap();
        std::fs::rename(&root, parent.path().join("replaced")).unwrap();
        std::fs::create_dir(&root).unwrap();

        assert_eq!(
            host.execute("file_read", json!({"path":"held.txt"}))
                .await
                .unwrap(),
            "held object"
        );
        assert!(
            host.execute("shell", json!({"command":"printf started > launched"}))
                .await
                .unwrap_err()
                .to_string()
                .contains("identity changed")
        );
        assert!(!root.join("launched").exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_environment_has_an_exact_independent_inventory() {
        use std::os::unix::ffi::OsStringExt;

        let expected = vec![
            ("PATH".into(), "/fixture/bin".into()),
            ("HOME".into(), "/fixture/home".into()),
            ("USER".into(), "fixture-user".into()),
            ("LOGNAME".into(), "fixture-login".into()),
            ("TMPDIR".into(), "/fixture/tmpdir".into()),
            ("TMP".into(), "/fixture/tmp".into()),
            ("TEMP".into(), "/fixture/temp".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("LC_COLLATE".into(), "C".into()),
            ("LC_CTYPE".into(), OsString::from_vec(vec![b'x', 0xff])),
            ("LC_MESSAGES".into(), "C".into()),
            ("LC_MONETARY".into(), "C".into()),
            ("LC_NUMERIC".into(), "C".into()),
            ("LC_TIME".into(), "C".into()),
            ("TZ".into(), "UTC".into()),
            ("NO_COLOR".into(), "1".into()),
            ("XDG_CONFIG_HOME".into(), "/fixture/config".into()),
            ("XDG_CACHE_HOME".into(), "/fixture/cache".into()),
            ("XDG_DATA_HOME".into(), "/fixture/data".into()),
            ("XDG_STATE_HOME".into(), "/fixture/state".into()),
            ("XDG_RUNTIME_DIR".into(), "/fixture/runtime".into()),
        ];
        let mut input = expected.clone();
        input.extend([
            ("OPENAI_API_KEY".into(), "fake-secret".into()),
            ("openai_api_key".into(), "also-ignored".into()),
            ("HTTP_PROXY".into(), "fake-proxy".into()),
            ("SSH_AUTH_SOCK".into(), "fake-agent".into()),
            ("GIT_ASKPASS".into(), "fake-askpass".into()),
            ("LD_LIBRARY_PATH".into(), "/fixture/loader".into()),
            ("BASH_ENV".into(), "fake-startup".into()),
            ("KURU_TEST_SENTINEL".into(), "fake-kuru".into()),
            ("UNRELATED_SHELL_FIXTURE".into(), "ignored".into()),
            (OsString::from_vec(vec![0xff]), "nonunicode-key".into()),
        ]);
        let projected = unix_shell_environment(input);

        assert_eq!(projected, expected);
        assert_eq!(projected[10].1.clone().into_vec(), vec![b'x', 0xff]);
        assert_eq!(
            unix_shell_environment([
                ("HOME".into(), "/fixture/home".into()),
                ("OPENAI_API_KEY".into(), "fake-secret".into()),
            ]),
            vec![(OsString::from("HOME"), OsString::from("/fixture/home"))]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_environment_has_an_exact_independent_inventory() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let system = Path::new(r"C:\Windows\System32");
        let expected = vec![
            ("PATH".into(), r"C:\fixture\bin".into()),
            ("HOME".into(), r"C:\fixture\home".into()),
            ("USER".into(), "fixture-user".into()),
            ("LOGNAME".into(), "fixture-login".into()),
            ("USERNAME".into(), "fixture-name".into()),
            ("USERPROFILE".into(), r"C:\fixture\profile".into()),
            ("HOMEDRIVE".into(), "C:".into()),
            ("HOMEPATH".into(), r"\fixture\profile".into()),
            ("APPDATA".into(), r"C:\fixture\appdata".into()),
            ("LOCALAPPDATA".into(), r"C:\fixture\localappdata".into()),
            ("ProgramData".into(), r"C:\fixture\programdata".into()),
            ("ProgramFiles".into(), r"C:\fixture\programfiles".into()),
            (
                "ProgramFiles(x86)".into(),
                r"C:\fixture\programfiles-x86".into(),
            ),
            ("ProgramW6432".into(), r"C:\fixture\programfiles-64".into()),
            ("PROCESSOR_ARCHITECTURE".into(), "AMD64".into()),
            ("PROCESSOR_ARCHITEW6432".into(), "AMD64".into()),
            ("TMPDIR".into(), r"C:\fixture\tmpdir".into()),
            ("TMP".into(), r"C:\fixture\tmp".into()),
            ("TEMP".into(), r"C:\fixture\temp".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("LC_COLLATE".into(), "C".into()),
            (
                "LC_CTYPE".into(),
                OsString::from_wide(&[0xd800, b'x' as u16]),
            ),
            ("LC_MESSAGES".into(), "C".into()),
            ("LC_MONETARY".into(), "C".into()),
            ("LC_NUMERIC".into(), "C".into()),
            ("LC_TIME".into(), "C".into()),
            ("TZ".into(), "UTC".into()),
            ("NO_COLOR".into(), "1".into()),
            ("XDG_CONFIG_HOME".into(), r"C:\fixture\config".into()),
            ("XDG_CACHE_HOME".into(), r"C:\fixture\cache".into()),
            ("XDG_DATA_HOME".into(), r"C:\fixture\data".into()),
            ("XDG_STATE_HOME".into(), r"C:\fixture\state".into()),
            ("XDG_RUNTIME_DIR".into(), r"C:\fixture\runtime".into()),
            ("PATHEXT".into(), ".EXE;.CMD".into()),
            ("SystemRoot".into(), r"C:\Windows".into()),
            ("WINDIR".into(), r"C:\Windows".into()),
            ("ComSpec".into(), r"C:\Windows\System32\cmd.exe".into()),
        ];
        let mut input: Vec<(OsString, OsString)> = expected[..35].to_vec();
        input[0].0 = "pAtH".into();
        input[5].0 = "userprofile".into();
        input[34].0 = "pAtHeXt".into();
        input.extend([
            ("PSMODULEPATH".into(), "hostile-one".into()),
            ("PsModulePath".into(), "hostile-two".into()),
            ("SystemRoot".into(), r"C:\hostile-one".into()),
            ("SYSTEMROOT".into(), r"C:\hostile-two".into()),
            ("WINDIR".into(), r"C:\hostile-windir-one".into()),
            ("windir".into(), r"C:\hostile-windir-two".into()),
            ("ComSpec".into(), r"C:\hostile-cmd-one.exe".into()),
            ("COMSPEC".into(), r"C:\hostile-cmd-two.exe".into()),
            ("OPENAI_API_KEY".into(), "fake-secret".into()),
            ("openai_api_key".into(), "also-ignored".into()),
            ("KURU_TEST_SENTINEL".into(), "fake-kuru".into()),
        ]);
        let projected = windows_shell_environment(input, system).unwrap();

        assert_eq!(projected, expected);
        assert_eq!(
            projected[22].1.encode_wide().collect::<Vec<_>>(),
            [0xd800, b'x' as u16]
        );
        assert_eq!(
            windows_shell_environment([("HOME".into(), r"C:\fixture\home".into())], system)
                .unwrap(),
            vec![
                (OsString::from("HOME"), OsString::from(r"C:\fixture\home")),
                (
                    OsString::from("PATHEXT"),
                    OsString::from(".COM;.EXE;.BAT;.CMD")
                ),
                (OsString::from("SystemRoot"), OsString::from(r"C:\Windows")),
                (OsString::from("WINDIR"), OsString::from(r"C:\Windows")),
                (
                    OsString::from("ComSpec"),
                    OsString::from(r"C:\Windows\System32\cmd.exe")
                ),
            ]
        );
        assert!(
            windows_shell_environment(
                [("Path".into(), "one".into()), ("PATH".into(), "two".into())],
                system,
            )
            .is_err()
        );
        assert!(
            windows_shell_environment(
                [
                    ("pAtHeXt".into(), ".EXE".into()),
                    ("PATHEXT".into(), ".CMD".into()),
                ],
                system,
            )
            .is_err()
        );
    }

    #[cfg(windows)]
    fn windows_shell_projection_source(root: &Path, case: &str) -> String {
        let home = root.join("home");
        let temporary = root.join("temporary");
        let commands = root.join("commands");
        let system = kuru_platform::windows::process::system_directory().unwrap();
        let windows = system.parent().unwrap();
        let expected_path = std::env::join_paths([commands.as_path(), system.as_path()])
            .expect("fixture paths must form a Windows PATH");
        let powershell_literal =
            |value: &OsStr| format!("'{}'", value.to_string_lossy().replace('\'', "''"));
        let expected_path = powershell_literal(&expected_path);
        let expected_home = powershell_literal(home.as_os_str());
        let expected_temporary = powershell_literal(temporary.as_os_str());
        let expected_windows = powershell_literal(windows.as_os_str());
        let expected_comspec = powershell_literal(system.join("cmd.exe").as_os_str());
        let expected_stage = powershell_literal(
            temporary
                .join(format!("shell-stage-{case}.txt"))
                .as_os_str(),
        );
        format!(
            r#"
[IO.File]::AppendAllText({expected_stage}, "entered`n")
$stage = {expected_stage}
$expectedPath = {expected_path}
$expectedHome = {expected_home}
$expectedTemporary = {expected_temporary}
$expectedWindows = {expected_windows}
$expectedComSpec = {expected_comspec}
# Kuru supplies the exact inherited value or fallback. Stock PowerShell then
# appends .CPL during engine construction when that extension is absent.
$expectedPathext = if ($env:NO_COLOR -eq 'inherited') {{ '.EXE;.CMD;.CPL' }} else {{ '.COM;.EXE;.BAT;.CMD;.CPL' }}
[IO.File]::AppendAllText($stage, "before-join-path`n")
$homePath = Join-Path $env:USERPROFILE 'shell-home.txt'
[IO.File]::AppendAllText($stage, "after-join-path`n")
[IO.File]::WriteAllText($homePath, 'home')
$temporaryPath = Join-Path $env:TEMP 'shell-temp.txt'
[IO.File]::WriteAllText($temporaryPath, 'temp')
[IO.File]::AppendAllText($stage, "home-temp-written`n")
[IO.File]::AppendAllText($stage, "before-where`n")
$where = & where.exe cmd.exe
$whereOk = $LASTEXITCODE -eq 0
[IO.File]::AppendAllText($stage, "after-where`n")
[IO.File]::AppendAllText($stage, "before-probe`n")
$probe = & probe
[IO.File]::AppendAllText($stage, "after-probe`n")
[IO.File]::AppendAllText($stage, "before-checks`n")
$checks = [ordered]@{{
    PATH = [string]::Equals($env:PATH, $expectedPath, [System.StringComparison]::Ordinal)
    HOME = [string]::Equals($env:HOME, $expectedHome, [System.StringComparison]::Ordinal)
    USERPROFILE = [string]::Equals($env:USERPROFILE, $expectedHome, [System.StringComparison]::Ordinal)
    TEMP = [string]::Equals($env:TEMP, $expectedTemporary, [System.StringComparison]::Ordinal)
    PATHEXT = [string]::Equals($env:PATHEXT, $expectedPathext, [System.StringComparison]::Ordinal)
    SystemRoot = [string]::Equals($env:SystemRoot, $expectedWindows, [System.StringComparison]::Ordinal)
    WINDIR = [string]::Equals($env:WINDIR, $expectedWindows, [System.StringComparison]::Ordinal)
    ComSpec = [string]::Equals($env:ComSpec, $expectedComSpec, [System.StringComparison]::Ordinal)
    OPENAI_API_KEY_absent = -not (Test-Path Env:OPENAI_API_KEY)
    HTTP_PROXY_absent = -not (Test-Path Env:HTTP_PROXY)
    PSModulePath_reconstructed = -not [string]::IsNullOrEmpty($env:PSModulePath)
    PSModulePath_hostile_absent = $env:PSModulePath -notlike '*fake-modules*'
    fixture_child_absent = -not (Test-Path Env:KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_CHILD)
    LLVM_PROFILE_FILE_absent = -not (Test-Path Env:LLVM_PROFILE_FILE)
    stock_cmdlet = (Get-Command Get-ChildItem -ErrorAction Stop).CommandType -eq 'Cmdlet'
    where_cmd = $whereOk
    probe_cmd = $probe -contains 'cmd-ok'
}}
[IO.File]::AppendAllText($stage, "after-checks`n")
$failed = @()
foreach ($check in $checks.GetEnumerator()) {{
    if (-not $check.Value) {{ $failed += [string]$check.Key }}
}}
if ($failed.Count -eq 0) {{
    [Console]::Out.Write('ok')
}} else {{
    throw ('shell compatibility fixture conditions failed: ' + [string]::Join(',', $failed))
}}
"#
        )
    }

    #[cfg(windows)]
    fn windows_shell_projection_stages(root: &Path, case: &str) -> String {
        let stage = root
            .join("temporary")
            .join(format!("shell-stage-{case}.txt"));
        let mut bytes = Vec::new();
        match std::fs::File::open(stage) {
            Ok(file) => {
                let mut limited = std::io::Read::take(file, 4096);
                match std::io::Read::read_to_end(&mut limited, &mut bytes) {
                    Ok(_) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(error) => format!("<unreadable: {error}>"),
                }
            }
            Err(error) => format!("<unavailable: {error}>"),
        }
    }

    #[cfg(windows)]
    pub(super) const WINDOWS_STACK_DIAGNOSTIC_SELECTOR: &str = "KURU_WINDOWS_STACK_DIAGNOSTIC";
    #[cfg(windows)]
    const WINDOWS_SHELL_FIXTURE_CHILD: &str = "KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_CHILD";
    #[cfg(windows)]
    const WINDOWS_CDB_PATH: &str = "KURU_WINDOWS_CDB_PATH";
    const CDB_OUTPUT_LIMIT: usize = 512 * 1024;
    #[cfg(windows)]
    const CDB_CAPTURE_LIMIT: Duration = Duration::from_secs(20);
    #[cfg(windows)]
    const CDB_CLEANUP_LIMIT: Duration = Duration::from_secs(5);
    #[cfg(windows)]
    const CDB_COMMANDS: &str = ".echo KURU_CDB_NATIVE_BEGIN\r\n~*e kc 80\r\n.echo KURU_CDB_NATIVE_END\r\n.loadby sos clr\r\n.echo KURU_CDB_MANAGED_BEGIN\r\n~*e !clrstack -n\r\n.echo KURU_CDB_MANAGED_END\r\nqd\r\n";

    #[cfg(windows)]
    pub(super) struct WindowsStackDiagnostic {
        pub(super) detail: String,
        pub(super) cleanup_confirmed: bool,
        command_file: Option<PathBuf>,
    }

    #[cfg(windows)]
    impl WindowsStackDiagnostic {
        pub(super) fn finish_after_target(&mut self, target_stopped: bool) {
            let Some(command_file) = self.command_file.take() else {
                return;
            };
            if !self.cleanup_confirmed || !target_stopped {
                self.command_file = Some(command_file);
                return;
            }
            if let Err(error) = std::fs::remove_file(&command_file) {
                self.cleanup_confirmed = false;
                use std::fmt::Write as _;
                let _ = write!(self.detail, "; debugger command cleanup failed: {error}");
                self.command_file = Some(command_file);
            }
        }
    }

    struct ParsedCdbStacks {
        detail: String,
        managed_sleep: bool,
    }

    fn cdb_command_echo(line: &str) -> bool {
        let Some((_, command)) = line.trim().rsplit_once('>') else {
            return false;
        };
        let command = command.trim_start();
        matches!(
            command,
            ".echo KURU_CDB_NATIVE_BEGIN"
                | "~*e kc 80"
                | ".echo KURU_CDB_NATIVE_END"
                | ".loadby sos clr"
                | ".echo KURU_CDB_MANAGED_BEGIN"
                | "~*e !clrstack -n"
                | ".echo KURU_CDB_MANAGED_END"
                | "qd"
        )
    }

    fn hexadecimal_token(token: &str, minimum: usize) -> bool {
        token.len() >= minimum && token.bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    fn native_stack_frame(line: &str) -> bool {
        let mut tokens = line.split_whitespace();
        let Some(first) = tokens.next() else {
            return false;
        };
        let frame = if hexadecimal_token(first, 1) && first.len() <= 3 {
            let Some(frame) = tokens.next() else {
                return false;
            };
            frame
        } else {
            first
        };
        if tokens.next().is_some() {
            return false;
        }
        let Some((module, function)) = frame.split_once('!') else {
            return false;
        };
        !module.is_empty() && !function.is_empty()
    }

    fn managed_stack_frame(line: &str) -> bool {
        let Some(address) = line.split_whitespace().next() else {
            return false;
        };
        hexadecimal_token(address, 8) && line.contains('(') && line.contains(')')
    }

    fn parse_cdb_stacks(stdout: &[u8]) -> Result<ParsedCdbStacks> {
        const NATIVE_BEGIN: &str = "KURU_CDB_NATIVE_BEGIN";
        const NATIVE_END: &str = "KURU_CDB_NATIVE_END";
        const MANAGED_BEGIN: &str = "KURU_CDB_MANAGED_BEGIN";
        const MANAGED_END: &str = "KURU_CDB_MANAGED_END";

        let output = std::str::from_utf8(stdout).context("CDB output is not UTF-8")?;
        let mut markers = [0_u8; 4];
        let mut state = 0_u8;
        let mut native = Vec::new();
        let mut managed = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            let marker = match trimmed {
                NATIVE_BEGIN => Some((0, 1)),
                NATIVE_END => Some((1, 2)),
                MANAGED_BEGIN => Some((2, 3)),
                MANAGED_END => Some((3, 4)),
                _ => None,
            };
            if let Some((index, next)) = marker {
                ensure!(state == index, "CDB stack markers are out of order");
                markers[index as usize] = markers[index as usize].saturating_add(1);
                ensure!(markers[index as usize] == 1, "duplicate CDB stack marker");
                state = next;
                continue;
            }
            if cdb_command_echo(line) {
                continue;
            }
            match state {
                1 => native.push(line),
                3 => managed.push(line),
                _ => {}
            }
        }
        ensure!(
            state == 4 && markers == [1, 1, 1, 1],
            "missing CDB stack marker"
        );

        let native = native.join("\n").trim().to_owned();
        let managed = managed.join("\n").trim().to_owned();
        ensure!(
            native.lines().any(native_stack_frame),
            "CDB native section has no module/function frame"
        );
        ensure!(
            managed.contains("OS Thread Id") && managed.lines().any(managed_stack_frame),
            "CDB managed section has no managed frame"
        );
        let lowered = managed.to_ascii_lowercase();
        ensure!(
            ![
                "failed to load",
                "failed to find runtime",
                "no export clrstack",
                "sos is not loaded",
            ]
            .iter()
            .any(|failure| lowered.contains(failure)),
            "CDB managed stack reports an SOS failure"
        );
        ensure!(
            native.len().saturating_add(managed.len()) <= CDB_OUTPUT_LIMIT,
            "parsed CDB stack sections exceed 512 KiB"
        );
        let managed_sleep = managed.contains("System.Threading.Thread.Sleep");
        Ok(ParsedCdbStacks {
            detail: format!("native stack:\n{native}\nmanaged stack:\n{managed}"),
            managed_sleep,
        })
    }

    fn windows_shell_cleanup_uncertain(stdout: &[u8], stderr: &[u8]) -> bool {
        [
            b"CDB cleanup unconfirmed".as_slice(),
            b"subprocess cleanup unconfirmed".as_slice(),
        ]
        .into_iter()
        .any(|marker| {
            stdout.windows(marker.len()).any(|bytes| bytes == marker)
                || stderr.windows(marker.len()).any(|bytes| bytes == marker)
        })
    }

    #[test]
    fn cdb_stack_parser_rejects_incomplete_or_unsafe_output() {
        let valid = concat!(
            "0:000> .echo KURU_CDB_NATIVE_BEGIN\r\n",
            "KURU_CDB_NATIVE_BEGIN\r\n",
            "ntdll!NtWaitForSingleObject+0x14\r\n",
            "0:000> .echo KURU_CDB_NATIVE_END\r\n",
            "KURU_CDB_NATIVE_END\r\n",
            "KURU_CDB_MANAGED_BEGIN\r\n",
            "OS Thread Id: 0x1234 (0)\r\n",
            "00000000 System.Threading.Thread.Sleep(Int32)\r\n",
            "KURU_CDB_MANAGED_END\r\n",
        );
        let parsed = parse_cdb_stacks(valid.as_bytes()).expect("valid bounded stacks");
        assert!(parsed.managed_sleep);
        assert!(!parsed.detail.contains(".echo KURU_CDB_NATIVE_BEGIN"));
        parse_cdb_stacks(valid.replace("ntdll!", "00 ntdll!").as_bytes())
            .expect("indexed native frame remains accepted");

        let failures = [
            ("missing", valid.replace("KURU_CDB_MANAGED_END\r\n", "")),
            (
                "duplicate",
                valid.replace(
                    "KURU_CDB_NATIVE_BEGIN\r\nntdll",
                    "KURU_CDB_NATIVE_BEGIN\r\nKURU_CDB_NATIVE_BEGIN\r\nntdll",
                ),
            ),
            (
                "out-of-order",
                valid.replace(
                    "KURU_CDB_NATIVE_END\r\nKURU_CDB_MANAGED_BEGIN",
                    "KURU_CDB_MANAGED_BEGIN\r\nKURU_CDB_NATIVE_END",
                ),
            ),
            (
                "echo-only",
                valid.replace(
                    "ntdll!NtWaitForSingleObject+0x14\r\n",
                    "0:000> ~*e kc 80\r\n",
                ),
            ),
            (
                "SOS-error",
                valid.replace(
                    "00000000 System.Threading.Thread.Sleep(Int32)",
                    "SOS is not loaded (failure)",
                ),
            ),
            (
                "managed-header-only",
                valid.replace("00000000 System.Threading.Thread.Sleep(Int32)\r\n", ""),
            ),
            (
                "native-header-only",
                valid.replace("ntdll!NtWaitForSingleObject+0x14\r\n", ""),
            ),
        ];
        for (name, output) in failures {
            assert!(
                parse_cdb_stacks(output.as_bytes()).is_err(),
                "{name} output was accepted"
            );
        }

        let overflow = valid.replace(
            "ntdll!NtWaitForSingleObject+0x14",
            &format!("ntdll!{}", "x".repeat(CDB_OUTPUT_LIMIT)),
        );
        assert!(parse_cdb_stacks(overflow.as_bytes()).is_err());
    }

    #[test]
    fn windows_shell_cleanup_uncertainty_markers_are_detected() {
        assert!(windows_shell_cleanup_uncertain(
            b"prefix CDB cleanup unconfirmed suffix",
            b""
        ));
        assert!(windows_shell_cleanup_uncertain(
            b"",
            b"prefix subprocess cleanup unconfirmed suffix"
        ));
        assert!(!windows_shell_cleanup_uncertain(
            b"subprocess tree terminated",
            b"ordinary failure"
        ));
    }

    #[cfg(windows)]
    async fn read_cdb_pipe(
        pipe: &mut kuru_platform::windows::pipe::Pipe,
    ) -> std::io::Result<(Vec<u8>, bool)> {
        let mut retained = Vec::new();
        let mut buffer = [0_u8; 8192];
        let mut overflow = false;
        loop {
            let count = pipe.read(&mut buffer).await?;
            if count == 0 {
                return Ok((retained, overflow));
            }
            let keep = count.min(CDB_OUTPUT_LIMIT.saturating_sub(retained.len()));
            retained.extend_from_slice(&buffer[..keep]);
            overflow |= keep != count;
        }
    }

    #[cfg(windows)]
    async fn cleanup_cdb(
        child: &mut kuru_platform::windows::process::NativeChild,
        stdout: &mut kuru_platform::windows::pipe::Pipe,
        stderr: &mut kuru_platform::windows::pipe::Pipe,
    ) -> bool {
        let requested = child.terminate().is_ok();
        let (stdout_close, stderr_close, reaped) = tokio::join!(
            stdout.close(CDB_CLEANUP_LIMIT),
            stderr.close(CDB_CLEANUP_LIMIT),
            child.wait(CDB_CLEANUP_LIMIT),
        );
        requested && stdout_close.is_ok() && stderr_close.is_ok() && reaped.is_ok()
    }

    #[cfg(windows)]
    async fn capture_windows_stack(
        target: &mut kuru_platform::windows::process::NativeChild,
        root: &Path,
        cdb_path: &Path,
    ) -> WindowsStackDiagnostic {
        use kuru_platform::windows::process::{Console, NativeSpawnSpec, Stdio, system_directory};

        let unavailable = |detail: String, command_file| WindowsStackDiagnostic {
            detail: format!("unavailable: {detail}"),
            cleanup_confirmed: true,
            command_file,
        };
        if !cdb_path.is_absolute() {
            return unavailable("CDB path is not absolute".into(), None);
        }
        let metadata = match std::fs::symlink_metadata(cdb_path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
            Ok(_) => return unavailable("CDB path is not a regular non-link file".into(), None),
            Err(error) => {
                return unavailable(format!("CDB path cannot be inspected: {error}"), None);
            }
        };
        if metadata.len() == 0 {
            return unavailable("CDB file is empty".into(), None);
        }
        let cdb_file = match std::fs::File::open(cdb_path) {
            Ok(file) => file,
            Err(error) => return unavailable(format!("CDB cannot be opened: {error}"), None),
        };
        if let Err(error) = regular_file_info(&cdb_file) {
            return unavailable(format!("CDB handle is not a regular file: {error}"), None);
        }

        let command_file = root.join("kuru-cdb-commands.txt");
        let mut commands = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&command_file)
        {
            Ok(file) => file,
            Err(error) => {
                return unavailable(format!("CDB command file cannot be created: {error}"), None);
            }
        };
        if let Err(error) = commands.write_all(CDB_COMMANDS.as_bytes()) {
            return unavailable(
                format!("CDB command file cannot be written: {error}"),
                Some(command_file),
            );
        }
        drop(commands);

        match target.try_wait() {
            Ok(None) => {}
            Ok(Some(_)) => {
                return unavailable(
                    "owned target exited before CDB attachment".into(),
                    Some(command_file),
                );
            }
            Err(error) => {
                return unavailable(
                    format!("owned target state cannot be observed: {error}"),
                    Some(command_file),
                );
            }
        }
        let pid = target.id();
        let system = match system_directory() {
            Ok(system) => system,
            Err(error) => {
                return unavailable(
                    format!("system directory is unavailable: {error}"),
                    Some(command_file),
                );
            }
        };
        let Some(windows) = system.parent() else {
            return unavailable(
                "system directory has no Windows parent".into(),
                Some(command_file),
            );
        };
        let mut spec = NativeSpawnSpec::new(cdb_path.to_path_buf(), root.to_path_buf());
        spec.args = vec![
            "-pv".into(),
            "-pd".into(),
            "-sins".into(),
            "-p".into(),
            pid.to_string().into(),
            "-cf".into(),
            command_file.as_os_str().to_owned(),
        ];
        spec.environment = vec![
            ("SystemRoot".into(), windows.as_os_str().to_owned()),
            ("WINDIR".into(), windows.as_os_str().to_owned()),
            ("ComSpec".into(), system.join("cmd.exe").into()),
            ("PATH".into(), system.as_os_str().to_owned()),
            ("TEMP".into(), root.as_os_str().to_owned()),
            ("TMP".into(), root.as_os_str().to_owned()),
        ];
        spec.console = Console::PrivateHidden;
        spec.stdout = Stdio::Pipe;
        spec.stderr = Stdio::Pipe;
        let mut cdb = match spec.spawn().await {
            Ok(child) => child,
            Err(error) => {
                return unavailable(format!("CDB launch failed: {error}"), Some(command_file));
            }
        };
        let stdout = cdb.take_stdout();
        let stderr = cdb.take_stderr();
        let (mut stdout, mut stderr) = match (stdout, stderr) {
            (Some(stdout), Some(stderr)) => (stdout, stderr),
            (stdout, stderr) => {
                let requested = cdb.terminate().is_ok();
                let stdout_close = async {
                    match stdout {
                        Some(mut stdout) => stdout.close(CDB_CLEANUP_LIMIT).await.is_ok(),
                        None => true,
                    }
                };
                let stderr_close = async {
                    match stderr {
                        Some(mut stderr) => stderr.close(CDB_CLEANUP_LIMIT).await.is_ok(),
                        None => true,
                    }
                };
                let (stdout_close, stderr_close, reaped) =
                    tokio::join!(stdout_close, stderr_close, cdb.wait(CDB_CLEANUP_LIMIT),);
                return WindowsStackDiagnostic {
                    detail: "unavailable: a CDB output pipe is missing".into(),
                    cleanup_confirmed: requested && stdout_close && stderr_close && reaped.is_ok(),
                    command_file: Some(command_file),
                };
            }
        };

        let outcome = timeout(CDB_CAPTURE_LIMIT, async {
            let (stdout, stderr, status) = tokio::join!(
                read_cdb_pipe(&mut stdout),
                read_cdb_pipe(&mut stderr),
                cdb.wait(CDB_CAPTURE_LIMIT),
            );
            let (stdout, stdout_overflow) = stdout?;
            let (stderr, stderr_overflow) = stderr?;
            ensure!(
                !stdout_overflow && !stderr_overflow,
                "CDB output exceeds its 512 KiB per-pipe limit"
            );
            let status = status?;
            ensure!(status.success(), "CDB exited unsuccessfully");
            Ok::<_, anyhow::Error>((stdout, stderr))
        })
        .await;

        match outcome {
            Ok(Ok((stdout_bytes, stderr_bytes))) => {
                let (stdout_close, stderr_close) = tokio::join!(
                    stdout.close(CDB_CLEANUP_LIMIT),
                    stderr.close(CDB_CLEANUP_LIMIT),
                );
                let cleanup_confirmed = stdout_close.is_ok() && stderr_close.is_ok();
                match parse_cdb_stacks(&stdout_bytes) {
                    Ok(parsed) => WindowsStackDiagnostic {
                        detail: format!(
                            "{}; CDB stderr={} bytes; managed-sleep={}",
                            parsed.detail,
                            stderr_bytes.len(),
                            parsed.managed_sleep
                        ),
                        cleanup_confirmed,
                        command_file: Some(command_file),
                    },
                    Err(error) => WindowsStackDiagnostic {
                        detail: format!(
                            "unavailable: {error:#}; CDB stdout={} bytes; stderr={} bytes",
                            stdout_bytes.len(),
                            stderr_bytes.len()
                        ),
                        cleanup_confirmed,
                        command_file: Some(command_file),
                    },
                }
            }
            Ok(Err(error)) => {
                let cleanup_confirmed = cleanup_cdb(&mut cdb, &mut stdout, &mut stderr).await;
                WindowsStackDiagnostic {
                    detail: format!("unavailable: {error:#}"),
                    cleanup_confirmed,
                    command_file: Some(command_file),
                }
            }
            Err(_) => {
                let cleanup_confirmed = cleanup_cdb(&mut cdb, &mut stdout, &mut stderr).await;
                WindowsStackDiagnostic {
                    detail: "unavailable: CDB capture timed out".into(),
                    cleanup_confirmed,
                    command_file: Some(command_file),
                }
            }
        }
    }

    #[cfg(windows)]
    pub(super) async fn capture_selected_windows_stack(
        target: &mut kuru_platform::windows::process::NativeChild,
        root: &Path,
    ) -> WindowsStackDiagnostic {
        if !windows_stack_diagnostic_selected() {
            return WindowsStackDiagnostic {
                detail: "unavailable: invalid diagnostic selector".into(),
                cleanup_confirmed: true,
                command_file: None,
            };
        }
        let Some(cdb_path) = std::env::var_os(WINDOWS_CDB_PATH) else {
            return WindowsStackDiagnostic {
                detail: "unavailable: checked CDB path was not supplied".into(),
                cleanup_confirmed: true,
                command_file: None,
            };
        };
        capture_windows_stack(target, root, Path::new(&cdb_path)).await
    }

    #[cfg(windows)]
    pub(super) fn windows_stack_diagnostic_selected() -> bool {
        std::env::var(WINDOWS_STACK_DIAGNOSTIC_SELECTOR).as_deref() == Ok("1")
            && std::env::var(WINDOWS_SHELL_FIXTURE_CHILD).as_deref() == Ok("1")
            && std::env::var("NO_COLOR").as_deref() == Ok("inherited")
    }

    #[cfg(windows)]
    async fn read_stack_target_ready(
        stdout: &mut kuru_platform::windows::pipe::Pipe,
    ) -> std::io::Result<()> {
        const READY: &[u8] = b"KURU_STACK_TARGET_READY\r\n";
        let mut observed = Vec::with_capacity(READY.len());
        while observed.len() < READY.len() {
            let mut byte = [0_u8; 1];
            let count = stdout.read(&mut byte).await?;
            if count == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "sleep control closed before readiness",
                ));
            }
            observed.push(byte[0]);
        }
        if observed != READY {
            return Err(std::io::Error::other(
                "sleep control emitted unexpected readiness bytes",
            ));
        }
        Ok(())
    }

    #[cfg(windows)]
    async fn windows_stack_known_sleep_control(cdb_path: &Path) -> Result<()> {
        use kuru_platform::windows::process::{Console, NativeSpawnSpec, Stdio, system_directory};

        let root = tempfile::tempdir()?;
        let system = system_directory()?;
        let windows = system
            .parent()
            .context("system directory has no Windows parent")?;
        let program = system.join("WindowsPowerShell/v1.0/powershell.exe");
        let mut spec = NativeSpawnSpec::new(program, root.path().to_path_buf());
        spec.args = [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-OutputFormat",
            "Text",
            "-Command",
            "[Console]::Out.WriteLine('KURU_STACK_TARGET_READY'); [Threading.Thread]::Sleep(120000)",
        ]
        .map(Into::into)
        .into();
        spec.environment = vec![
            ("SystemRoot".into(), windows.as_os_str().to_owned()),
            ("WINDIR".into(), windows.as_os_str().to_owned()),
            ("ComSpec".into(), system.join("cmd.exe").into()),
            ("PATH".into(), system.as_os_str().to_owned()),
            ("TEMP".into(), root.path().as_os_str().to_owned()),
            ("TMP".into(), root.path().as_os_str().to_owned()),
        ];
        spec.console = Console::Inherit;
        spec.stdout = Stdio::Pipe;
        spec.stderr = Stdio::Pipe;
        let mut target = spec.spawn().await.context("sleep control launch failed")?;
        let stdout = target.take_stdout();
        let stderr = target.take_stderr();
        let (mut stdout, mut stderr) = match (stdout, stderr) {
            (Some(stdout), Some(stderr)) => (stdout, stderr),
            (stdout, stderr) => {
                let requested = target.terminate().is_ok();
                let stdout_close = async {
                    match stdout {
                        Some(mut stdout) => stdout.close(CDB_CLEANUP_LIMIT).await.is_ok(),
                        None => true,
                    }
                };
                let stderr_close = async {
                    match stderr {
                        Some(mut stderr) => stderr.close(CDB_CLEANUP_LIMIT).await.is_ok(),
                        None => true,
                    }
                };
                let (stdout_close, stderr_close, reaped) =
                    tokio::join!(stdout_close, stderr_close, target.wait(CDB_CLEANUP_LIMIT),);
                if requested && stdout_close && stderr_close && reaped.is_ok() {
                    bail!("sleep control output pipe is missing");
                }
                let preserved = root.keep();
                bail!(
                    "sleep control output pipe and cleanup failed; preserved {}",
                    preserved.display()
                );
            }
        };

        let ready = timeout(
            Duration::from_secs(10),
            read_stack_target_ready(&mut stdout),
        )
        .await;
        if !matches!(ready, Ok(Ok(()))) {
            let requested = target.terminate();
            let (stdout_close, stderr_close, reaped) = tokio::join!(
                stdout.close(CDB_CLEANUP_LIMIT),
                stderr.close(CDB_CLEANUP_LIMIT),
                target.wait(CDB_CLEANUP_LIMIT),
            );
            if requested.is_err()
                || stdout_close.is_err()
                || stderr_close.is_err()
                || reaped.is_err()
            {
                let preserved = root.keep();
                bail!(
                    "sleep control readiness and cleanup failed; preserved {}",
                    preserved.display()
                );
            }
            bail!("sleep control did not reach readiness: {ready:?}");
        }

        let mut diagnostic = capture_windows_stack(&mut target, root.path(), cdb_path).await;
        let still_running = matches!(target.try_wait(), Ok(None));
        let requested = target.terminate();
        let (stdout_close, stderr_close, reaped) = tokio::join!(
            stdout.close(CDB_CLEANUP_LIMIT),
            stderr.close(CDB_CLEANUP_LIMIT),
            target.wait(CDB_CLEANUP_LIMIT),
        );
        let target_cleanup =
            requested.is_ok() && stdout_close.is_ok() && stderr_close.is_ok() && reaped.is_ok();
        diagnostic.finish_after_target(target_cleanup);
        if !diagnostic.cleanup_confirmed || !target_cleanup {
            let preserved = root.keep();
            bail!(
                "sleep control cleanup remains unconfirmed; CDB={}; target={}; preserved {}",
                diagnostic.cleanup_confirmed,
                target_cleanup,
                preserved.display()
            );
        }
        ensure!(
            still_running,
            "sleep control target exited during noninvasive capture"
        );
        ensure!(
            diagnostic.detail.contains("managed-sleep=true"),
            "sleep control did not capture System.Threading.Thread.Sleep: {}",
            diagnostic.detail
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_toolhost_shell_receives_only_compatibility_environment() {
        const CHILD: &str = "KURU_SHELL_ENVIRONMENT_TEST_CHILD";
        const ROOT: &str = "KURU_SHELL_ENVIRONMENT_TEST_ROOT";
        if std::env::var_os(CHILD).is_some() {
            let root = PathBuf::from(std::env::var_os(ROOT).expect("missing test root"));
            let home = root.join("home");
            let temporary = root.join("temporary");
            let cache = root.join("cache");
            let quote = |path: &Path| {
                format!(
                    "'{}'",
                    path.as_os_str().to_string_lossy().replace('\'', "'\"'\"'")
                )
            };
            let expected_home = quote(&home);
            let expected_temporary = quote(&temporary);
            let expected_cache = quote(&cache);
            let shell = [
                "test \"$PATH\" = '/usr/bin:/bin' && test \"$HOME\" = ",
                &expected_home,
                " && test \"$TMPDIR\" = ",
                &expected_temporary,
                " && test \"$XDG_CACHE_HOME\" = ",
                &expected_cache,
                " && test \"$LC_CTYPE\" = 'C.UTF-8' && test -z \"${OPENAI_API_KEY+x}\" && test -z \"${HTTP_PROXY+x}\" && test -z \"${SSH_AUTH_SOCK+x}\" && test -z \"${GIT_ASKPASS+x}\" && test -z \"${LD_LIBRARY_PATH+x}\" && test -z \"${BASH_ENV+x}\" && test -z \"${KURU_SHELL_ENVIRONMENT_TEST_CHILD+x}\" && test -z \"${UNRELATED_SHELL_FIXTURE+x}\" && test -z \"${LLVM_PROFILE_FILE+x}\" && touch cwd-write \"$HOME/home-write\" \"$TMPDIR/temp-write\" && test -f cwd-write && test -f \"$HOME/home-write\" && test -f \"$TMPDIR/temp-write\" && printf ok",
            ]
            .concat();
            let host = ToolHost::new(
                &root,
                &Config {
                    allow_shell: true,
                    ..Config::default()
                },
            )
            .unwrap();
            let output: Value = serde_json::from_str(
                &host
                    .execute("shell", json!({"command":shell}))
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert!(
                output["success"] == true && output["stdout"] == "ok",
                "shell compatibility environment was not minimized"
            );
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let temporary = root.path().join("temporary");
        let cache = root.path().join("cache");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&temporary).unwrap();
        std::fs::create_dir(&cache).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg("tools::tests::isolated_toolhost_shell_receives_only_compatibility_environment")
            .arg("--nocapture")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("TMPDIR", &temporary)
            .env("LC_CTYPE", "C.UTF-8")
            .env("XDG_CACHE_HOME", &cache)
            .env("OPENAI_API_KEY", "fake-api-key")
            .env("HTTP_PROXY", "fake-proxy")
            .env("SSH_AUTH_SOCK", "fake-agent")
            .env("GIT_ASKPASS", "fake-askpass")
            .env("LD_LIBRARY_PATH", "/fake/loader")
            .env("BASH_ENV", "fake-startup")
            .env("UNRELATED_SHELL_FIXTURE", "ignored")
            .env(CHILD, "1")
            .env(ROOT, root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut child = command.spawn().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let result = timeout(Duration::from_secs(10), async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let (stdout_truncated, stderr_truncated, status) = tokio::join!(
                drain_bounded(&mut stdout, &mut out),
                drain_bounded(&mut stderr, &mut err),
                child.wait(),
            );
            if stdout_truncated? || stderr_truncated? {
                return Err(std::io::Error::other(
                    "isolated shell fixture output exceeds 2 MiB",
                ));
            }
            Ok::<_, std::io::Error>((status?, out, err))
        })
        .await;
        let (status, out, err) = match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                let requested = child.start_kill();
                let (stdout_drain, stderr_drain, reap) = tokio::join!(
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stdout, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stderr, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), child.wait()),
                );
                let cleanup_confirmed = stdout_drain.as_ref().is_ok_and(|result| result.is_ok())
                    && stderr_drain.as_ref().is_ok_and(|result| result.is_ok())
                    && reap.as_ref().is_ok_and(|result| result.is_ok());
                if !cleanup_confirmed {
                    let preserved = root.keep();
                    panic!(
                        "isolated shell environment fixture failed: {error}; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}; reap={reap:?}; preserved {}",
                        preserved.display()
                    );
                }
                panic!(
                    "isolated shell environment fixture failed: {error}; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}"
                );
            }
            Err(_) => {
                let requested = child.start_kill();
                let (stdout_drain, stderr_drain, reap) = tokio::join!(
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stdout, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stderr, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), child.wait()),
                );
                let cleanup_confirmed = stdout_drain.as_ref().is_ok_and(|result| result.is_ok())
                    && stderr_drain.as_ref().is_ok_and(|result| result.is_ok())
                    && reap.as_ref().is_ok_and(|result| result.is_ok());
                if !cleanup_confirmed {
                    let preserved = root.keep();
                    panic!(
                        "isolated shell environment fixture timed out; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}; reap={reap:?}; preserved {}",
                        preserved.display()
                    );
                }
                panic!(
                    "isolated shell environment fixture timed out; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}"
                );
            }
        };
        assert!(
            status.success(),
            "isolated shell environment fixture failed: stdout={} stderr={}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
        assert!(root.path().join("cwd-write").is_file());
        assert!(home.join("home-write").is_file());
        assert!(temporary.join("temp-write").is_file());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn isolated_toolhost_windows_shell_receives_projected_environment() {
        use kuru_platform::windows::process::{NativeSpawnSpec, Stdio, system_directory};

        const CHILD: &str = "KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_CHILD";
        const ROOT: &str = "KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_ROOT";

        if std::env::var_os(CHILD).is_some() {
            let root = PathBuf::from(std::env::var_os(ROOT).expect("missing test root"));
            let home = root.join("home");
            let temporary = root.join("temporary");
            let case = match std::env::var("NO_COLOR").as_deref() {
                Ok("inherited") => "inherited",
                Ok("fallback") => "fallback",
                value => panic!("unexpected Windows shell fixture case: {value:?}"),
            };
            let stage_trace = || windows_shell_projection_stages(&root, case);
            let host = ToolHost::new(
                &root,
                &Config {
                    allow_shell: true,
                    ..Config::default()
                },
            )
            .unwrap();
            let shell = windows_shell_projection_source(&root, case);
            let receipt = host
                .execute("shell", json!({"command":shell}))
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "Windows shell projection fixture failed: {error:#}; stages={}",
                        stage_trace()
                    )
                });
            let output: Value = serde_json::from_str(&receipt).unwrap_or_else(|error| {
                panic!(
                    "Windows shell projection fixture returned invalid receipt: {error}; receipt={receipt}; stages={}",
                    stage_trace()
                )
            });
            assert!(
                output["success"] == true && output["stdout"] == "ok",
                "Windows shell projection fixture failed: receipt={output}; stages={}",
                stage_trace()
            );
            assert_eq!(
                stage_trace(),
                "entered\nbefore-join-path\nafter-join-path\nhome-temp-written\nbefore-where\nafter-where\nbefore-probe\nafter-probe\nbefore-checks\nafter-checks\n"
            );
            assert_eq!(
                std::fs::read_to_string(home.join("shell-home.txt")).unwrap(),
                "home"
            );
            assert_eq!(
                std::fs::read_to_string(temporary.join("shell-temp.txt")).unwrap(),
                "temp"
            );
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let commands = root.path().join("commands");
        let home = root.path().join("home");
        let temporary = root.path().join("temporary");
        let local = root.path().join("local");
        let roaming = root.path().join("roaming");
        std::fs::create_dir(&commands).unwrap();
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&temporary).unwrap();
        std::fs::create_dir(&local).unwrap();
        std::fs::create_dir(&roaming).unwrap();
        std::fs::write(commands.join("probe.cmd"), "@echo cmd-ok\r\n").unwrap();
        let system = system_directory().unwrap();
        let path = std::env::join_paths([commands.as_path(), system.as_path()]).unwrap();
        let cdb_path = std::env::var_os(WINDOWS_CDB_PATH).map(PathBuf::from);
        let mut failure = None;
        let mut preserve_root = false;
        for (name, pathext) in [("inherited", Some(".EXE;.CMD")), ("fallback", None)] {
            let mut spec =
                NativeSpawnSpec::new(std::env::current_exe().unwrap(), root.path().into());
            spec.args = vec![
                "--exact".into(),
                "tools::tests::isolated_toolhost_windows_shell_receives_projected_environment"
                    .into(),
                "--nocapture".into(),
            ];
            spec.environment = vec![
                ("pAtH".into(), path.clone()),
                ("HOME".into(), home.clone().into()),
                ("userprofile".into(), home.clone().into()),
                ("LOCALAPPDATA".into(), local.clone().into()),
                ("APPDATA".into(), roaming.clone().into()),
                ("TEMP".into(), temporary.clone().into()),
                ("TMP".into(), temporary.clone().into()),
                ("NO_COLOR".into(), name.into()),
                ("SystemRoot".into(), r"C:\hostile-system".into()),
                ("WINDIR".into(), r"C:\hostile-windir".into()),
                ("ComSpec".into(), r"C:\hostile-cmd.exe".into()),
                ("OPENAI_API_KEY".into(), "fake-api-key".into()),
                ("HTTP_PROXY".into(), "fake-proxy".into()),
                ("pSmOdUlEpAtH".into(), "fake-modules".into()),
                (CHILD.into(), "1".into()),
                (ROOT.into(), root.path().into()),
            ];
            for key in [
                "PROCESSOR_ARCHITECTURE",
                "PROCESSOR_ARCHITEW6432",
                "ProgramData",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramW6432",
            ] {
                if let Some(value) = std::env::var_os(key) {
                    spec.environment.push((key.into(), value));
                }
            }
            if let Some(pathext) = pathext {
                spec.environment.push(("pAtHeXt".into(), pathext.into()));
            }
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                spec.environment.push(("LLVM_PROFILE_FILE".into(), profile));
            }
            let diagnostic_armed = name == "inherited" && cdb_path.is_some();
            if diagnostic_armed {
                spec.environment
                    .push((WINDOWS_STACK_DIAGNOSTIC_SELECTOR.into(), "1".into()));
                spec.environment.push((
                    WINDOWS_CDB_PATH.into(),
                    cdb_path
                        .as_ref()
                        .expect("armed diagnostic has a CDB path")
                        .as_os_str()
                        .to_owned(),
                ));
            }
            spec.stdout = Stdio::Pipe;
            spec.stderr = Stdio::Pipe;
            let mut child = spec.spawn().await.unwrap();
            let mut stdout = child.take_stdout().unwrap();
            let mut stderr = child.take_stderr().unwrap();
            let mut out = Vec::new();
            let mut err = Vec::new();
            let parent_limit = if diagnostic_armed { 75 } else { 45 };
            let child_limit = if diagnostic_armed { 70 } else { 40 };
            let outcome = timeout(Duration::from_secs(parent_limit), async {
                let (stdout_truncated, stderr_truncated, status) = tokio::join!(
                    drain_bounded(&mut stdout, &mut out),
                    drain_bounded(&mut stderr, &mut err),
                    child.wait(Duration::from_secs(child_limit)),
                );
                let stdout_truncated = stdout_truncated?;
                let stderr_truncated = stderr_truncated?;
                if stdout_truncated || stderr_truncated {
                    return Err(std::io::Error::other(
                        "Windows shell fixture output exceeds 2 MiB",
                    ));
                }
                Ok::<_, std::io::Error>(status?)
            })
            .await;
            let status = match outcome {
                Ok(Ok(status)) => Ok(status),
                Ok(Err(error)) => {
                    let requested = child.terminate();
                    let (stdout_close, stderr_close) = tokio::join!(
                        stdout.close(Duration::from_secs(5)),
                        stderr.close(Duration::from_secs(5)),
                    );
                    let reap = child.wait(Duration::from_secs(5)).await;
                    if stdout_close.is_err() || stderr_close.is_err() || reap.is_err() {
                        preserve_root = true;
                    }
                    Err(format!(
                        "Windows shell fixture {name} failed: {error}; kill={requested:?}; stdout-close={stdout_close:?}; stderr-close={stderr_close:?}; reap={reap:?}; stderr-prefix={}",
                        String::from_utf8_lossy(&err[..err.len().min(4096)])
                    ))
                }
                Err(_) => {
                    let requested = child.terminate();
                    let (stdout_close, stderr_close) = tokio::join!(
                        stdout.close(Duration::from_secs(5)),
                        stderr.close(Duration::from_secs(5)),
                    );
                    let reap = child.wait(Duration::from_secs(5)).await;
                    if stdout_close.is_err() || stderr_close.is_err() || reap.is_err() {
                        preserve_root = true;
                    }
                    Err(format!(
                        "Windows shell fixture {name} timed out; kill={requested:?}; stdout-close={stdout_close:?}; stderr-close={stderr_close:?}; reap={reap:?}; stderr-prefix={}",
                        String::from_utf8_lossy(&err[..err.len().min(4096)])
                    ))
                }
            };
            let status = match status {
                Ok(status) => status,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            };
            if !status.success() {
                preserve_root |= windows_shell_cleanup_uncertain(&out, &err);
                failure = Some(format!(
                    "Windows shell fixture {name} failed: stdout={} stderr={}",
                    String::from_utf8_lossy(&out),
                    String::from_utf8_lossy(&err)
                ));
                break;
            }
        }
        let control = match &cdb_path {
            Some(cdb_path) => windows_stack_known_sleep_control(cdb_path).await,
            None => Ok(()),
        };
        if let Some(failure) = failure {
            if preserve_root {
                let preserved = root.keep();
                panic!(
                    "{failure}; known-sleep={control:?}; preserved {}",
                    preserved.display()
                );
            }
            panic!("{failure}; known-sleep={control:?}");
        }
        control.expect("known-sleep CDB control failed");
    }
}
