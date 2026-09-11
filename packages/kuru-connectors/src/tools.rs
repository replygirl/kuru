use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use kuru_core::{Config, ToolSpec};
#[cfg(windows)]
use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info, validate_component};
use serde_json::{Value, json};
#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use tokio::process::Command;
use tokio::{io::AsyncReadExt, time::timeout};
#[cfg(windows)]
type PathGuard = Directory;
#[cfg(unix)]
type PathGuard = ();

use crate::{MAX_BYTES, mcp::McpHosts};

/// File tools operate under an opened directory capability. Shell and MCP
/// authorization grant process/server authority; cwd is not an OS sandbox.
pub struct ToolHost {
    root: PathBuf,
    directory: Dir,
    #[cfg(windows)]
    root_guard: Directory,
    allow_write: bool,
    allow_shell: bool,
    mcp: McpHosts,
}

impl ToolHost {
    pub fn new(root: &Path, config: &Config) -> Result<Self> {
        let root = root.canonicalize().context("tool root does not exist")?;
        #[cfg(windows)]
        let root_guard = Directory::open(&root, Privacy::Inherited, NameRetention::Pinned)?;
        let directory = Dir::open_ambient_dir(&root, ambient_authority())?;
        Ok(Self {
            mcp: McpHosts::new(&root, &config.mcp)?,
            root,
            directory,
            #[cfg(windows)]
            root_guard,
            allow_write: config.allow_write,
            allow_shell: config.allow_shell,
        })
    }

    pub async fn specs(&self) -> Result<Vec<ToolSpec>> {
        let mut specs = vec![
            spec(
                "file_read",
                "Read a UTF-8 project file (2 MiB limit).",
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
                "Execute a shell command with process authority, in the project cwd. This is not a filesystem sandbox. Output is capped; timeout is at most 120 seconds.",
                &["command"],
                &["command"],
            );
            shell.parameters["properties"]["timeout_ms"] =
                json!({"type":"integer","minimum":1,"maximum":120000});
            specs.push(shell);
        }
        specs.extend(self.mcp.specs().await?);
        Ok(specs)
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        ensure!(args.is_object(), "tool arguments must be an object");
        match name {
            "file_read" => {
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
                let mut bytes = Vec::new();
                file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= MAX_BYTES, "file exceeds 2 MiB limit");
                String::from_utf8(bytes).context("file is not UTF-8")
            }
            "file_write" => {
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
                Ok(format!("Wrote {} bytes", content.len()))
            }
            "file_delete" => {
                ensure!(self.allow_write, "file deletion requires allow_write=true");
                let (directory, path, _guard) = self.path(string(&args, "path")?, true)?;
                ensure!(
                    directory.symlink_metadata(&path)?.is_file(),
                    "file_delete requires a regular file"
                );
                directory.remove_file(path)?;
                Ok("Deleted file".into())
            }
            "file_list" => {
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
                Ok(serde_json::to_string(&entries)?)
            }
            "shell" => {
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
                shell(
                    &self.root,
                    string(&args, "command")?,
                    Duration::from_millis(duration),
                )
                .await
            }
            _ => self.mcp.execute(name, args).await,
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.mcp.shutdown().await
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
pub(crate) struct ProcessGroup(pub(crate) Option<u32>);

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

#[cfg(unix)]
async fn shell(root: &Path, command: &str, duration: Duration) -> Result<String> {
    ensure!(!command.trim().is_empty(), "shell command is empty");
    let mut process = Command::new("sh");
    process
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    process.process_group(0);
    let mut child = process.spawn().context("cannot start shell")?;
    let _group = ProcessGroup(child.id());
    let stdout = child.stdout.take().context("missing shell stdout")?;
    let stderr = child.stderr.take().context("missing shell stderr")?;
    let operation = async {
        let read = async |reader: Box<dyn tokio::io::AsyncRead + Unpin + Send>| -> Result<Vec<u8>> {
            let mut bytes = Vec::new();
            reader
                .take((MAX_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(bytes.len() <= MAX_BYTES, "shell output exceeds 2 MiB limit");
            Ok(bytes)
        };
        let (out, err) = tokio::try_join!(read(Box::new(stdout)), read(Box::new(stderr)))?;
        let status = child.wait().await?;
        Ok::<_, anyhow::Error>(json!({"exit_code":status.code(),"success":status.success(),"stdout":String::from_utf8_lossy(&out),"stderr":String::from_utf8_lossy(&err)}).to_string())
    };
    timeout(duration, operation)
        .await
        .context("shell timed out; process group terminated")?
}

#[cfg(windows)]
async fn shell(root: &Path, command: &str, duration: Duration) -> Result<String> {
    use base64::Engine;
    use kuru_platform::windows::process::{Stdio, configured_command, system_directory};
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
    let program = system_directory()?.join("WindowsPowerShell/v1.0/powershell.exe");
    let mut spec = configured_command(
        program.as_os_str(),
        &args,
        root,
        std::env::vars_os().collect(),
    )?;
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    let mut child = spec
        .spawn()
        .await
        .context("cannot start Windows PowerShell")?;
    let mut stdout = child.take_stdout().context("missing shell stdout")?;
    let mut stderr = child.take_stderr().context("missing shell stderr")?;
    let operation = async {
        async fn read(reader: &mut kuru_platform::windows::pipe::Pipe) -> Result<Vec<u8>> {
            let mut bytes = Vec::new();
            reader
                .take((MAX_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(bytes.len() <= MAX_BYTES, "shell output exceeds 2 MiB limit");
            Ok(bytes)
        }
        let (out, err) = tokio::try_join!(read(&mut stdout), read(&mut stderr))?;
        let status = child.wait(duration).await?;
        Ok::<_, anyhow::Error>(json!({"exit_code":status.code(),"success":status.success(),"stdout":String::from_utf8_lossy(&out),"stderr":String::from_utf8_lossy(&err)}).to_string())
    };
    let result = timeout(duration, operation)
        .await
        .context("shell timed out; subprocess tree terminated")
        .and_then(|result| result);
    if result.is_err() {
        let _ = crate::process::stop(&mut child).await;
    }
    let cleanup = async {
        stdout.close(Duration::from_secs(5)).await?;
        stderr.close(Duration::from_secs(5)).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    match result {
        Ok(value) => {
            cleanup?;
            Ok(value)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn permissions_and_size_limits_reject_before_mutation() {
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
        std::fs::write(root.path().join("large"), vec![b'a'; MAX_BYTES + 1]).unwrap();
        std::fs::write(root.path().join("binary"), [0xff]).unwrap();
        assert!(
            read_only
                .execute("file_read", json!({"path":"large"}))
                .await
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
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
        let (command, stall, flood) = (
            "printf hello; printf problem >&2; exit 7",
            "sleep 5",
            "yes output",
        );
        #[cfg(windows)]
        let (command, stall, flood) = (
            "[Console]::Out.Write('hello'); [Console]::Error.Write('problem'); exit 7",
            "Start-Sleep -Seconds 5",
            "[Console]::Out.Write('x' * 2097153)",
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
        assert!(
            host.execute("shell", json!({"command":flood}))
                .await
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
    }
}
