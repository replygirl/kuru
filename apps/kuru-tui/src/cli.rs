use std::{
    ffi::OsStr,
    fs::File,
    future::Future,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::ui::theme::{self, Role};
use anyhow::{Context, Result, bail, ensure};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use kuru_connectors::permissions::{PermissionBinding, PermissionService};
use kuru_connectors::{
    CheckpointStore, McpAvailability, McpCatalogStore, McpCredentialStore, McpStatus, Provider,
    ToolHost, provider,
};
use kuru_core::UiConfig;

fn human_stdout(ui: &UiConfig, role: Role, line: &str) {
    let terminal = io::stdout().is_terminal();
    #[cfg(windows)]
    let terminal = terminal
        && kuru_platform::windows::console::virtual_terminal_output_enabled(
            kuru_platform::windows::process::StandardStream::Output,
        );
    println!("{}", theme::human_status(line, ui, role, terminal));
}

fn human_stderr(ui: &UiConfig, role: Role, line: &str) {
    let terminal = io::stderr().is_terminal();
    #[cfg(windows)]
    let terminal = terminal
        && kuru_platform::windows::console::virtual_terminal_output_enabled(
            kuru_platform::windows::process::StandardStream::Error,
        );
    eprintln!("{}", theme::human_status(line, ui, role, terminal));
}

use kuru_core::{
    AuthorityClaimCategory, Config, ConfigDisplayBounds, ConfigSnapshot, InvocationOverrides, Mode,
    ModelInfo, ProjectPreferences, SafeManifest,
};
use kuru_memory::{
    CandidateRefRejected, CandidateRefState, MemoryStore, OpenOptions as MemoryOptions,
    SelectedAbandonResolution, SelectedAbandonUncertain,
};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use kuru_runtime::{
    Event, HOOK_ANNOTATION_UNRESOLVED_AFTER_ANSWER, Harness, forget_note, read_notes,
};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{
    commands, memory_export,
    permission_store::GrantStore,
    trust::{ApprovalState, ApprovalStore},
};

pub use crate::headless::OutputFormat;

#[derive(Debug, Parser)]
#[command(
    name = "kuru",
    version,
    about = "A conversation with a pool of persistent peers"
)]
pub struct Cli {
    #[arg(short = 'C', long, global = true, default_value = ".")]
    pub directory: PathBuf,
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[arg(short = 'c', global = true, value_name = "KEY=VALUE", action = clap::ArgAction::Append)]
    pub config_values: Vec<String>,
    #[arg(long, global = true, env = "KURU_DATA_DIR")]
    pub data_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub provider: Option<String>,
    #[arg(long, global = true)]
    pub mode: Option<Mode>,
    #[arg(long, global = true)]
    pub model: Option<String>,
    #[arg(long, global = true)]
    pub effort: Option<String>,
    #[arg(long, global = true)]
    pub resume: Option<String>,
    #[arg(long = "continue", global = true, conflicts_with = "resume")]
    pub continue_session: bool,
    #[arg(
        long,
        global = true,
        help = "Allow workspace file mutations unless an explicit permission rule restricts them"
    )]
    pub allow_write: bool,
    #[arg(
        long,
        global = true,
        help = "Allow shell unless an explicit permission rule restricts it (process authority, not a sandbox)"
    )]
    pub allow_shell: bool,
    #[arg(long, global = true)]
    pub no_dream: bool,
    #[arg(
        long,
        global = true,
        help = "Record additional bounded local operational diagnostics"
    )]
    pub debug: bool,
    #[arg(
        long,
        global = true,
        help = "Authorize this command's reviewed workspace authority without saving approval"
    )]
    pub trust_workspace_once: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print shell completions derived from the current command tree.
    Completions {
        shell: CompletionShell,
        /// Use a separately installed Usage executable for completion answers.
        #[arg(long)]
        external_usage: bool,
    },
    /// Print the current command manual in roff format.
    Man,
    /// Execute a prompt without the terminal UI.
    Run {
        prompt: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, value_enum, conflicts_with = "json")]
        output_format: Option<OutputFormat>,
        /// Use an explicit durable ID for exact retry in this session.
        #[arg(long, value_name = "ID")]
        turn_id: Option<String>,
    },
    /// Sign in to ChatGPT using native OpenAI authentication.
    Login {
        #[arg(long)]
        device: bool,
        /// Print the browser URL without opening it automatically.
        #[arg(long, conflicts_with = "device")]
        no_browser: bool,
    },
    Logout,
    /// Show authentication status without displaying tokens.
    Auth,
    /// Inspect local configuration, trust, authentication, memory, and bundled engine status.
    Doctor {
        /// Emit the fixed diagnostic report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Check the fixed ChatGPT subscription route without opening workspace tools or memory.
    Canary {
        /// Model to use for the bounded no-tool request.
        #[arg(long)]
        model: Option<String>,
    },
    /// Discover provider models and supported reasoning efforts.
    Models,
    /// Print merged effective configuration.
    Config,
    /// List or change durable sessions without invoking a provider.
    Sessions {
        #[command(subcommand)]
        command: Option<SessionCommand>,
    },
    /// Inspect this project's memory store and revision history.
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    /// Consolidate private memories and consider reversible membership changes.
    Dream,
    UndoDream,
    /// Invoke a workspace/MCP tool directly.
    Tool {
        name: String,
        #[arg(long, default_value = "{}")]
        args: String,
    },
    /// Inspect, undo, or explicitly prune this project's private file checkpoints.
    File {
        #[command(subcommand)]
        command: FileCommand,
    },
    /// List available workspace and MCP tools.
    Tools,
    /// Sign in, inspect, or sign out of one configured MCP server.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Serve authenticated A2A 1.0 on loopback.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7437")]
        bind: std::net::SocketAddr,
        #[arg(long, default_value = "KURU_A2A_TOKEN")]
        token_env: String,
    },
    /// Install a verified explicit release, or rebuild from a local checkout.
    Update {
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        release_base: Option<String>,
        #[arg(long)]
        source: Option<PathBuf>,
    },
    /// Inspect or change approval for automatic workspace configuration.
    Trust {
        #[command(subcommand)]
        command: TrustCommand,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
}

impl CompletionShell {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
        }
    }

    fn native_shell(self) -> usage_argv::complete::Shell {
        match self {
            Self::Bash => usage_argv::complete::Shell::Bash,
            Self::Zsh => usage_argv::complete::Shell::Zsh,
            Self::Fish => usage_argv::complete::Shell::Fish,
            Self::PowerShell => usage_argv::complete::Shell::PowerShell,
        }
    }
}

const SHELL_SUPPORT_OUTPUT_LIMIT: usize = 512 * 1024;

fn usage_spec() -> usage::Spec {
    let mut command = Cli::command();
    command.build();
    usage::Spec::from(&command)
}

fn shell_support_output(command: &Command) -> Result<Vec<u8>> {
    let output = match command {
        Command::Completions {
            shell,
            external_usage,
        } => {
            if *external_usage {
                usage::complete::complete(&usage::complete::CompleteOptions {
                    usage_bin: "usage".to_owned(),
                    shell: shell.as_str().to_owned(),
                    bin: "kuru".to_owned(),
                    cache_key: Some(env!("CARGO_PKG_VERSION").to_owned()),
                    spec: Some(usage_spec()),
                    usage_cmd: None,
                    source_file: None,
                })?
                .into_bytes()
            } else {
                usage_argv::script::script("kuru", shell.native_shell()).into_bytes()
            }
        }
        Command::Man => usage::docs::manpage::ManpageRenderer::new(usage_spec())
            .render()?
            .into_bytes(),
        _ => bail!("shell-support output requires a generation command"),
    };
    ensure!(
        !output.is_empty() && output.len() <= SHELL_SUPPORT_OUTPUT_LIMIT,
        "generated shell support exceeds its bounded output limit"
    );
    Ok(output)
}

/// Process-launching audit (AGENTS.md: "audit new process-launching
/// dependencies and consumer call sites instead of claiming isolation across
/// arbitrary spawn mechanisms"), against `usage-cli` 6.11.1 as published:
///
/// - `usage::sh::sh` (tries `sh -c` first on every platform, falling back to
///   `cmd /c` only on Windows and only when `sh` itself is not found) runs
///   behind `usage_cli::complete_answer` only when a `SpecComplete.run` is
///   `Some(_)` (`usage-cli/src/cli/complete_word.rs`, `if let Some(run) =
///   &complete.run`). `usage-lib`'s `From<&clap::Command> for Spec`
///   conversion (`usage-lib/src/spec/cmd.rs`, the arg/flag loop building
///   `spec.complete`) only ever sets `name` and `type_` from
///   `clap::ValueHint`; `run` is left at its `Default` (`None`) for every
///   entry. `usage_spec()` below is exactly that conversion, so no
///   `SpecComplete` it produces can ever carry a `run` script — this path is
///   statically unreachable here, not merely unexercised.
/// - `std::process::Command` at `usage-cli/src/cli/exec.rs:92` and
///   `cli/shell.rs:133` belong to `usage-cli`'s own `exec`/`shell`
///   subcommands (`Cli::Exec`, `Cli::Shell`), reachable only through
///   `usage_cli::Cli`'s own arg dispatch (`Cli::run`/`Cli::parse_from`).
///   Kuru never imports `usage_cli::Cli` and never calls `usage_cli::run`;
///   the only `usage_cli` symbol referenced anywhere in this crate is
///   `complete_answer` itself (grepped: `usage_cli::` appears exactly once,
///   at its call site below).
/// - The Unix-only `exec` crate (`[target.'cfg(unix)'.dependencies]` in
///   `usage-cli`'s Cargo.toml) has no call site under its `src/` at all as
///   published — `exec::` never appears in its source, only the crate's own
///   local `mod exec` (the module above, an unrelated name collision). It
///   appears to be a vestigial manifest entry upstream; either way nothing
///   in `usage-cli`'s compiled surface invokes it.
/// - `env_logger`'s `Builder::init()` call lives only in `usage-cli`'s
///   `[[bin]] usage` target (`src/main.rs`), which is never built when
///   `usage-cli` is consumed as a library dependency, as Kuru does.
///
/// Net: the only process this call path can start is Kuru's own re-exec of
/// itself through the shell-installed completion script, never a subprocess
/// `usage-cli` spawns on Kuru's behalf.
fn native_completion_answer_output(
    request: &usage_argv::complete::CompletionRequest,
) -> Result<Vec<u8>> {
    let words = request.split.walked();
    ensure!(
        words.len() <= 128 && words.iter().map(String::len).sum::<usize>() <= 16 * 1024,
        "completion request exceeds its bound"
    );
    let answer = usage_cli::complete_answer(
        &usage_spec(),
        words,
        request.split.cword,
        request.shell.as_str(),
    )?;
    let mut projected = usage_argv::complete::Completions::default();
    if answer.files && !request.split.prefix.starts_with("--") {
        projected.files = Some(usage_argv::complete::Files::Any);
    } else {
        projected.candidates = answer
            .candidates
            .into_iter()
            .map(|(value, description)| {
                if description.is_empty() {
                    usage_argv::complete::Candidate::new(value)
                } else {
                    usage_argv::complete::Candidate::described(value, description)
                }
            })
            .collect();
    }
    let output = usage_argv::complete::render_request(&projected, request);
    ensure!(
        output.len() <= SHELL_SUPPORT_OUTPUT_LIMIT,
        "completion answer exceeds its bounded output limit"
    );
    Ok(output.into_bytes())
}

fn native_completion_from_argv() -> Result<Option<Vec<u8>>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    usage_argv::complete::CompletionRequest::parse(&args)
        .map(|request| native_completion_answer_output(&request))
        .transpose()
}

#[cfg(test)]
mod shell_support_tests {
    use super::*;

    #[test]
    fn usage_completion_protocol_preserves_unicode_spaces_and_quotes() {
        let candidates = || usage_argv::complete::Completions {
            candidates: vec![
                usage_argv::complete::Candidate::described("plain", "simple"),
                usage_argv::complete::Candidate::described("équipe d'amis", "Unicode choice"),
            ],
            files: None,
        };
        let request = |shell: &str| {
            usage_argv::complete::CompletionRequest::parse(&[
                "__complete_word__".into(),
                "--shell".into(),
                shell.into(),
                "--line".into(),
                "kuru ".into(),
            ])
            .unwrap()
        };
        for shell in ["bash", "fish", "zsh", "powershell"] {
            let output = usage_argv::complete::render_request(&candidates(), &request(shell));
            assert!(output.contains("équipe d'amis"), "{shell}: {output}");
            if shell != "bash" {
                assert!(output.contains("Unicode choice"), "{shell}: {output}");
            }
            if shell == "zsh" {
                assert!(output.contains("'équipe d'\\''amis'"), "{output}");
            }
        }
    }

    #[test]
    fn native_answer_is_not_an_authored_clap_command() {
        assert!(
            usage_spec()
                .cmd
                .subcommands
                .get("__complete_word__")
                .is_none()
        );
    }
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Sign in to one configured OAuth-enabled MCP server.
    Login {
        alias: String,
        /// Use the server's advertised device authorization flow.
        #[arg(long)]
        device: bool,
        /// Print the browser URL without opening it automatically.
        #[arg(long, conflicts_with = "device")]
        no_browser: bool,
    },
    /// Inspect local sign-in state without displaying credentials.
    Status { alias: String },
    /// Delete the local credential after a bounded remote revocation attempt.
    Logout { alias: String },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Rename one active or removed session.
    Rename { session: String, label: String },
    /// Remove one session from ordinary listing and resume selection.
    Remove { session: String },
    /// Restore one reversibly removed session.
    Restore { session: String },
    /// Fork an immutable settled public prefix into a new session.
    Fork {
        session: String,
        node: String,
        #[arg(long)]
        child_id: Option<String>,
        #[arg(long, default_value = "Fork")]
        label: String,
    },
    /// Export one session's public transcript without private memory.
    Export {
        session: String,
        #[arg(long, value_enum, default_value_t = SessionExportFormat::Markdown)]
        format: SessionExportFormat,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum SessionExportFormat {
    Jsonl,
    Markdown,
}

#[derive(Debug, Subcommand)]
pub enum TrustCommand {
    /// Show current automatic workspace authority and approval state.
    Status,
    /// Review and persist approval for the complete current authority manifest.
    Approve {
        /// Approve noninteractively after printing the complete redacted manifest.
        #[arg(long)]
        yes: bool,
    },
    /// Remove this exact workspace's saved approval.
    Revoke,
}

#[derive(Debug, Subcommand)]
pub enum FileCommand {
    /// List bounded checkpoint metadata without file snapshot bodies.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Inspect one selected checkpoint without its private snapshots.
    Inspect { id: String },
    /// Undo one durably applied file effect if its checked post-state still matches.
    Undo { id: String },
    /// Remove one inactive receipt; unresolved effects require explicit discard.
    Prune {
        id: String,
        #[arg(long)]
        discard_uncertain: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ExportFormat {
    Json,
    Markdown,
}

#[derive(Debug)]
pub struct CanaryExitCode(pub u8);

impl std::fmt::Display for CanaryExitCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "canary reported exit status {}", self.0)
    }
}

impl std::error::Error for CanaryExitCode {}

pub fn canary_exit_code(error: &anyhow::Error) -> Option<i32> {
    error
        .downcast_ref::<CanaryExitCode>()
        .map(|exit| i32::from(exit.0))
}

pub fn doctor_exit_code(error: &anyhow::Error) -> Option<i32> {
    error
        .downcast_ref::<crate::doctor::DoctorExit>()
        .map(|exit| exit.0)
}

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// List bounded project scopes in the legacy SQLite source without opening Dolt.
    Inventory,
    /// Import one inventoried source scope into this canonical project.
    Import {
        /// Opaque source scope from `kuru memory inventory`; omit for exact-scope import.
        #[arg(long)]
        source_scope: Option<String>,
    },
    /// Show the active project, engine version and revision.
    Status,
    /// List recent committed memory revisions.
    History {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// List bounded retained dream candidate refs.
    Candidates {
        #[arg(long, default_value_t = commands::CANDIDATE_PAGE_LIMIT)]
        limit: usize,
        #[arg(long)]
        after: Option<String>,
    },
    /// Recheck one exact retained candidate ref.
    CandidateStatus { branch: String },
    /// Explicitly abandon one exact candidate ref after inspection.
    CandidateAbandon {
        branch: String,
        #[arg(long)]
        base: String,
        #[arg(long)]
        head: String,
    },
    /// Export every application record from one committed active-memory snapshot.
    Export {
        #[arg(long, value_enum, default_value_t = ExportFormat::Json)]
        format: ExportFormat,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Read one peer or relationship's durable notes without starting a conversation.
    Notes {
        identity: String,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Remove one selected current note while retaining prior Dolt revisions.
    Forget {
        identity: String,
        #[arg(long)]
        note: i64,
    },
    /// Explicitly remove this project's managed Dolt memory and history.
    ///
    /// Original/shared legacy SQLite inputs and migration snapshots, exports,
    /// backups, other projects, engine cache, and stable locks remain. The
    /// selected project's diagnostics ring is removed after its memory. If a
    /// prior purge stopped after recording its intent, rerun this command to
    /// remove only its recorded remaining identities.
    Purge {
        /// Confirm removal of this project's local current memory and all managed revisions.
        #[arg(long)]
        yes: bool,
    },
}

pub fn paths(cli: &Cli) -> Result<(PathBuf, PathBuf, Option<PathBuf>)> {
    let cwd = cli
        .directory
        .canonicalize()
        .context("workspace directory does not exist")?;
    ensure!(cwd.is_dir(), "workspace must be a directory");
    let user_config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(native_config_directory)
        .map(|path| path.join("kuru/config.toml"))
        .filter(|path| path.exists());
    let data = cli
        .data_dir
        .clone()
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(native_data_directory)
                .map(|path| path.join("kuru"))
        })
        .context("no user data directory is available; set --data-dir or KURU_DATA_DIR")?;
    let data = if data.is_absolute() {
        data
    } else {
        std::env::current_dir()?.join(data)
    };
    Ok((cwd, data, user_config))
}

pub(crate) fn native_config_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| {
        std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("AppData/Roaming"))
    });
    #[cfg(unix)]
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

fn native_data_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("AppData/Local"))
        });
    #[cfg(unix)]
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
}

pub(crate) fn invocation_overrides(cli: &Cli) -> InvocationOverrides {
    InvocationOverrides {
        typed_config: cli.config_values.clone(),
        mode: cli.mode,
        provider: cli.provider.clone(),
        model: cli.model.clone(),
        effort: cli.effort.clone(),
        allow_write: cli.allow_write,
        allow_shell: cli.allow_shell,
        no_dream: cli.no_dream,
    }
}

/// A local file is user authority only when it is demonstrably absent from a
/// repository index. This query is fixed and read-only; it never invokes shell.
pub(crate) async fn discovered_local(root: &Directory) -> Result<Option<(PathBuf, String)>> {
    let local = root.path().join(".kuru/config.local.toml");
    let metadata = match std::fs::symlink_metadata(&local) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("project-local configuration cannot be inspected"),
    };
    ensure!(
        metadata.is_file(),
        "project-local configuration must be a regular file"
    );
    let parent = Directory::open(
        &root.path().join(".kuru"),
        Privacy::Inherited,
        NameRetention::Pinned,
    )
    .context("project-local configuration directory is unsafe")?;
    let mut file = parent
        .read(OsStr::new("config.local.toml"))
        .context("project-local configuration file is unsafe")?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .context("project-local configuration cannot be read")?;
    ensure!(
        bytes.len() <= 256 * 1024,
        "project-local configuration exceeds the 256 KiB file limit"
    );
    let captured = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("project-local configuration is not UTF-8"))?;
    parent
        .verify(OsStr::new("config.local.toml"), &file)
        .context("project-local configuration changed during inspection")?;
    root.revalidate()
        .context("workspace changed during local configuration inspection")?;

    let repository = root
        .path()
        .ancestors()
        .any(|ancestor| ancestor.join(".git").exists());
    if repository {
        let mut command = tokio::process::Command::new("git");
        command
            .arg("--no-optional-locks")
            .args([
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=/nonexistent",
            ])
            .arg("-C")
            .arg(root.path())
            .args([
                "ls-files",
                "--error-unmatch",
                "--",
                ".kuru/config.local.toml",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        // Inherited Git selectors can redirect -C to another index and make a
        // tracked project file appear untracked. Keep the caller's ordinary
        // process environment while removing Git's repository/config overlays.
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
        // A silent `ls-files` reports progress only by exiting, so no derived
        // ceiling exists: a large index or slow disk legitimately takes long.
        // Known hang sources (fsmonitor, hooks, optional locks, Git overlays)
        // are removed above, and kill_on_drop reaps the child when the user
        // cancels the command.
        let status = command
            .status()
            .await
            .map_err(|_| anyhow::anyhow!("project-local Git index check is unavailable"))?;
        match status.code() {
            Some(0) => bail!(
                "Git tracks .kuru/config.local.toml; remove it from the index or use --config explicitly"
            ),
            Some(1) => {}
            _ => bail!("project-local Git index status is ambiguous; use --config explicitly"),
        }
    }
    parent
        .verify(OsStr::new("config.local.toml"), &file)
        .context("project-local configuration changed during Git inspection")?;
    Ok(Some((local, captured)))
}

pub async fn select_model(
    config: &mut Config,
    provider: &Arc<dyn Provider>,
) -> Result<Vec<ModelInfo>> {
    let models = provider.models().await?;
    if config.model == "auto" {
        ensure!(
            config.provider != "responses",
            "the Responses model catalog does not advertise a default chat model; select --model MODEL_ID from kuru --provider responses models"
        );
        let model = models
            .first()
            .context("provider returned no models; set an explicit model")?;
        config.model = model.id.clone();
        if config.effort.is_none() {
            config.effort = model.default_effort.clone();
        }
    }
    validate_effort(&models, &config.model, config.effort.as_deref())?;
    Ok(models)
}

pub fn validate_effort(models: &[ModelInfo], model: &str, effort: Option<&str>) -> Result<()> {
    if let (Some(model), Some(effort)) = (models.iter().find(|m| m.id == model), effort) {
        ensure!(
            model.efforts.is_empty() || model.efforts.iter().any(|e| e == effort),
            "effort '{effort}' unsupported by {}; available: {}",
            model.id,
            model.efforts.join(", ")
        );
    }
    Ok(())
}

async fn open_memory(
    options: MemoryOptions,
    project: &Path,
    interactive: bool,
) -> Result<MemoryStore> {
    crate::memory_activity::start_clock();
    let markers = crate::memory_activity::markers_enabled(
        std::env::var_os(crate::memory_activity::MARKERS_ENV).as_deref(),
    );
    let configured_cache = options.config.cache_dir.is_some();
    let executable =
        kuru_platform::running_executable().context("locate the running Kuru executable")?;
    let (progress, opening) =
        MemoryStore::open_managed_observed(options, project.to_owned(), executable);
    let mut output = crate::memory_activity::ActivityOutput::for_process(interactive, markers);
    crate::memory_activity::drive(opening, progress, &mut output, configured_cache).await
}

async fn settle_selected_candidate_abandon(
    memory: &MemoryStore,
    primary: anyhow::Error,
) -> Result<()> {
    let recovered = tokio::time::timeout(std::time::Duration::from_secs(35), async {
        loop {
            match memory.recover_selected_candidate_abandon().await {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => {
                    bail!("selected candidate abandonment has no retained request identity")
                }
                Err(error) if error.is::<SelectedAbandonUncertain>() => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await;
    match recovered {
        Ok(Ok(SelectedAbandonResolution::Abandoned)) => Ok(()),
        Ok(Ok(outcome)) => Err(primary.context(format!(
            "selected candidate abandonment resolved as {outcome:?}"
        ))),
        Ok(Err(recovery)) => Err(primary.context(format!(
            "selected candidate outcome recovery failed: {recovery:#}"
        ))),
        Err(_) => Err(primary.context("selected candidate outcome recovery deadline exceeded")),
    }
}

fn data_directory_error(data: &Path, error: std::io::Error) -> anyhow::Error {
    #[cfg(unix)]
    if error.kind() == io::ErrorKind::PermissionDenied
        && let Ok(directory) = std::fs::symlink_metadata(data)
        && directory.file_type().is_dir()
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if directory.uid() == nix::unistd::geteuid().as_raw()
            && directory.permissions().mode() & 0o077 != 0
        {
            return anyhow::Error::from(error).context(format!(
                "memory data directory {data:?} is not owner-private; restrict this exact directory to mode 0700 (for example with chmod, using shell quoting) and retry"
            ));
        }
    }
    #[cfg(windows)]
    if error.kind() == io::ErrorKind::PermissionDenied {
        return anyhow::Error::from(error).context(format!(
            "memory data directory {data:?} is not owner-private; correct this directory's owner-only access with Windows file security settings and retry"
        ));
    }
    error.into()
}

/// Writes generated shell-support or completion-answer bytes to stdout, the
/// same as a well-behaved Unix tool: a reader that closes early (`kuru
/// completions bash | head`) is not an error condition — Rust's runtime
/// ignores `SIGPIPE`, so a write past a closed pipe returns a plain
/// `BrokenPipe` `io::Error` rather than terminating the process, and letting
/// that propagate as an ordinary `anyhow` error prints a spurious `Error:
/// Broken pipe (os error 32)` and exits 1 for what is, from the reader's
/// side, a completely successful invocation.
fn write_stdout_ignoring_broken_pipe(output: &[u8]) -> Result<()> {
    match io::stdout().write_all(output) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub async fn run() -> Result<()> {
    if let Some(output) = native_completion_from_argv()? {
        return write_stdout_ignoring_broken_pipe(&output);
    }
    execute(Cli::parse()).await
}

/// Binary-only entrypoint. Library callers remain subscriber-neutral.
pub async fn run_with_diagnostics() -> Result<()> {
    if let Some(output) = native_completion_from_argv()? {
        return write_stdout_ignoring_broken_pipe(&output);
    }
    execute_inner(Cli::parse(), true).await
}

pub async fn execute(cli: Cli) -> Result<()> {
    execute_inner(cli, false).await
}

async fn execute_inner(mut cli: Cli, install_diagnostics: bool) -> Result<()> {
    let mut invocation_signal = None;
    // Input completes before paths, configuration/trust or any configured
    // provider, tool, memory or diagnostic authority can activate.
    if let Some(Command::Run {
        prompt, turn_id, ..
    }) = &mut cli.command
    {
        if turn_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        {
            return Err(anyhow::Error::new(crate::headless::RunExit(2))
                .context("turn ID must contain 1–256 bytes"));
        }
        let (input, signal) = crate::headless::read_input(prompt.take()).await?;
        *prompt = Some(input);
        invocation_signal = Some(signal);
    }
    #[cfg(unix)]
    if !matches!(cli.command, Some(Command::Update { .. })) {
        recover_unix_installation(&mut invocation_signal).await?;
    }
    if let Some(command @ (Command::Completions { .. } | Command::Man)) = &cli.command {
        return write_stdout_ignoring_broken_pipe(&shell_support_output(command)?);
    }
    if let Some(Command::Doctor { json }) = &cli.command {
        return crate::doctor::run_from_cli(&cli, *json).await;
    }
    if let Some(Command::Update {
        version,
        release_base,
        source,
    }) = &cli.command
    {
        return update(
            version.as_deref(),
            release_base.as_deref(),
            source.as_deref(),
        )
        .await;
    }
    let (cwd, data, user) = paths(&cli)?;
    let root = Arc::new(
        Directory::open(&cwd, Privacy::Inherited, NameRetention::Movable)
            .context("workspace directory could not be retained safely")?,
    );
    let cwd = root.path().to_path_buf();

    // Inventory is a fixed, non-provisioning inspection. It must not parse or
    // activate automatic workspace configuration to list old scopes.
    if matches!(
        cli.command,
        Some(Command::Memory {
            command: MemoryCommand::Inventory,
        })
    ) {
        let inventory = match MemoryStore::legacy_inventory(&data) {
            Ok(inventory) => inventory,
            Err(error) => {
                if let Some(refusal) = error.downcast_ref::<kuru_memory::LegacyInventoryRefusal>() {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "refusal": refusal,
                        }))?
                    );
                }
                return Err(error.context(
                    "legacy inventory could not be verified; preserve the source and inspect it",
                ));
            }
        };
        println!("{}", serde_json::to_string_pretty(&inventory)?);
        return Ok(());
    }

    // Login and logout are fixed ChatGPT account operations. In particular,
    // they neither parse workspace-selected Responses configuration nor read
    // its environment variable.
    if matches!(cli.command, Some(Command::Login { .. } | Command::Logout)) {
        return crate::authentication::run(
            cli.command
                .as_ref()
                .expect("matched authentication command"),
            None,
            &data,
            &cwd,
        )
        .await;
    }

    if let Some(Command::Canary { model }) = &cli.command {
        let model = model
            .as_deref()
            .context("kuru canary requires --model MODEL")?;
        let report = crate::authentication::canary(model, &data, &cwd).await;
        println!("{}", serde_json::to_string_pretty(&report)?);
        let exit = match report.state {
            kuru_connectors::CanaryState::Verified => 0,
            kuru_connectors::CanaryState::Incompatible => 3,
            kuru_connectors::CanaryState::Unverified => 2,
        };
        if exit != 0 {
            return Err(CanaryExitCode(exit).into());
        }
        return Ok(());
    }

    // Revocation must remain possible when current configuration is malformed.
    if matches!(
        cli.command,
        Some(Command::Trust {
            command: TrustCommand::Revoke
        })
    ) {
        let removed = ApprovalStore::new(&data, &root).revoke()?;
        println!(
            "{}",
            if removed {
                "Workspace approval revoked."
            } else {
                "No workspace approval was stored."
            }
        );
        return Ok(());
    }

    // Checkpoint inspection and selected pruning use only Kuru's checked
    // private store. Malformed or unapproved project configuration must not
    // activate or obstruct these explicit recovery commands.
    if let Some(Command::File { command }) = &cli.command {
        match command {
            FileCommand::List { limit } => {
                ensure!(
                    (1..=1000).contains(limit),
                    "checkpoint list limit must be 1–1000"
                );
                let rows = CheckpointStore::existing(&data, root.clone())?
                    .map(|store| store.list(*limit))
                    .transpose()?
                    .unwrap_or_default();
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(());
            }
            FileCommand::Inspect { id } => {
                let store = CheckpointStore::existing(&data, root.clone())?
                    .context("file checkpoint does not exist")?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &store
                            .inspect(id)?
                            .context("file checkpoint does not exist")?
                    )?
                );
                return Ok(());
            }
            FileCommand::Prune {
                id,
                discard_uncertain,
            } => {
                let store = CheckpointStore::existing(&data, root.clone())?
                    .context("file checkpoint does not exist")?;
                let prior = store
                    .inspect(id)?
                    .context("file checkpoint does not exist")?;
                ensure!(
                    store.prune(id, *discard_uncertain)?,
                    "file checkpoint disappeared before pruning"
                );
                println!(
                    "{}",
                    if prior.state == kuru_connectors::CheckpointState::Applied {
                        "Selected settled file checkpoint pruned; its undo and exact-retry evidence is no longer available."
                    } else {
                        "Selected unresolved file checkpoint discarded; its recovery and undo evidence is no longer available."
                    }
                );
                return Ok(());
            }
            FileCommand::Undo { .. } => {}
        }
    }

    let local = discovered_local(&root).await?;
    let managed = std::env::var_os("KURU_MANAGED_CONFIG").map(PathBuf::from);
    let user_prompt_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(native_config_directory)
        .map(|path| path.join("kuru"));
    let snapshot = ConfigSnapshot::parse_with_sources(
        user.as_deref(),
        user_prompt_root.as_deref(),
        &cwd,
        local
            .as_ref()
            .map(|(path, content)| (path.as_path(), content.as_str())),
        cli.config.as_deref(),
        managed.as_deref(),
        &crate::commands::built_in_names(),
        invocation_overrides(&cli),
    )?;
    for notice in snapshot.instruction_notices() {
        eprintln!("{notice}");
    }
    for notice in snapshot.prompt_catalog().notices() {
        eprintln!("{notice}");
    }
    root.revalidate()
        .context("workspace changed while configuration was being reviewed")?;
    let approval_store = ApprovalStore::new(&data, &root);

    if let Some(Command::Trust { command }) = &cli.command {
        match command {
            TrustCommand::Status => {
                show_manifest(
                    &root,
                    &snapshot.manifest().filtered(&all_claim_categories()),
                    Some(approval_store.inspect(snapshot.manifest())),
                );
            }
            TrustCommand::Approve { yes } => {
                let complete = snapshot.manifest().filtered(&all_claim_categories());
                show_manifest(&root, &complete, None);
                if !yes && !confirm("Approve this complete workspace authority manifest? [y/N] ")? {
                    bail!("workspace approval cancelled");
                }
                root.revalidate()
                    .context("workspace changed before approval was recorded")?;
                approval_store.approve_command(snapshot.manifest())?;
                println!("Complete workspace authority manifest approved.");
            }
            TrustCommand::Revoke => {
                unreachable!("revocation returned before configuration parsing")
            }
        }
        return Ok(());
    }

    if matches!(cli.command, Some(Command::Config)) {
        println!("{}", snapshot.snapshot_toml()?);
        eprintln!(
            "note: saved project mode, model, and effort preferences are omitted; memory was not opened"
        );
        return Ok(());
    }

    // Past this check an interactive session has terminal stdin and stdout.
    let interactive = cli.command.is_none();
    if interactive && !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        bail!("interactive mode requires a terminal; use kuru run PROMPT");
    }

    preflight(&cli, &root, &data, &snapshot)?;
    root.revalidate()
        .context("workspace changed after trust preflight")?;

    if matches!(cli.command, Some(Command::Auth)) {
        let route = snapshot.responses_route()?;
        let result = crate::authentication::run(
            cli.command
                .as_ref()
                .expect("matched authentication command"),
            route.as_ref().map(|route| route.api_key_env()),
            &data,
            &cwd,
        )
        .await;
        if route.is_none() {
            eprintln!(
                "note: Responses API-key availability was not checked; select --provider responses to inspect that route"
            );
        }
        return result;
    }

    // Explicit provider/model catalog inspection does not need saved choices
    // and must return before any project-memory or legacy activation.
    if explicitly_selected_models(&cli) {
        let config = snapshot.finalize(&ProjectPreferences::default())?;
        let provider = provider(&config, &cwd, &data).await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&provider.models().await?)?
        );
        return Ok(());
    }

    // Catalog inspection needs the reviewed tool/MCP authority, but it must
    // remain independent of project memory and its configured runtime. Use
    // defaults for memory-backed preferences and return before deriving a
    // memory scope, lease, or store path.
    if matches!(cli.command, Some(Command::Tools)) {
        let config = snapshot.finalize(&ProjectPreferences::default())?;
        let host = permission_host(&data, root.clone(), &config, &snapshot, false)?;
        let catalog = host.catalog().await;
        let cleanup = host.shutdown().await;
        let printed = catalog.and_then(|catalog| {
            report_mcp_statuses(catalog.mcp(), &config.ui);
            println!("{}", serde_json::to_string_pretty(&catalog)?);
            Ok(())
        });
        return finish(printed, cleanup, "tool host");
    }

    if let Some(Command::Mcp { command }) = &cli.command {
        let config = snapshot.finalize(&ProjectPreferences::default())?;
        let host = permission_host(&data, root.clone(), &config, &snapshot, false)?;
        let result = run_mcp_command(&host, command, &config.ui).await;
        let cleanup = host.shutdown().await;
        return finish(result, cleanup, "tool host");
    }

    let scope = kuru_runtime::project_scope(&cwd)?;
    let memory_config = snapshot.memory_config().clone();
    if let Some(Command::Memory {
        command: MemoryCommand::Import { source_scope },
    }) = &cli.command
    {
        let _retained_data = Directory::open(&data, Privacy::OwnerOnly, NameRetention::Movable)
            .map_err(|error| data_directory_error(&data, error))?;
        ensure_outside_workspace(&data, &cwd)?;
        root.revalidate()
            .context("workspace changed before explicit legacy import")?;
        let mut options = MemoryOptions::new(data.clone(), scope.clone());
        options.config = memory_config;
        let operation = MemoryStore::import_legacy(options, source_scope.clone());
        let outcome = match finish_memory_operation(operation, || {}, &mut invocation_signal).await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                if let Some(rejected) = error.downcast_ref::<kuru_memory::LegacyImportRejected>() {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "refusal": rejected.0,
                        }))?
                    );
                }
                return Err(error.context(
                    "legacy import did not complete; inspect the target before retrying",
                ));
            }
        };
        println!("{}", serde_json::to_string_pretty(&outcome)?);
        return Ok(());
    }
    let session_writer = matches!(
        &cli.command,
        Some(Command::Sessions {
            command: Some(
                SessionCommand::Rename { .. }
                    | SessionCommand::Remove { .. }
                    | SessionCommand::Restore { .. }
                    | SessionCommand::Fork { .. }
            )
        })
    );
    let writer = session_writer
        || matches!(
            cli.command,
            None | Some(
                Command::Run { .. }
                    | Command::Dream
                    | Command::UndoDream
                    | Command::Serve { .. }
                    | Command::Memory {
                        command: MemoryCommand::Forget { .. }
                            | MemoryCommand::CandidateAbandon { .. }
                            | MemoryCommand::Purge { .. }
                    }
            )
        );
    let runtime_owner = matches!(
        cli.command,
        None | Some(
            Command::Run { .. } | Command::Dream | Command::UndoDream | Command::Serve { .. }
        )
    );
    let purge = matches!(
        cli.command,
        Some(Command::Memory {
            command: MemoryCommand::Purge { .. }
        })
    );
    if let Some(Command::Memory {
        command: MemoryCommand::Purge { yes: false },
    }) = &cli.command
    {
        bail!(
            "memory purge removes this project's local Dolt history and current memory; rerun with --yes after reviewing the retained-history boundary"
        );
    }
    let exists = if purge {
        false
    } else {
        MemoryStore::exists(&data, &scope)?
    };
    let memory_control = matches!(
        cli.command,
        Some(Command::Memory {
            command: MemoryCommand::Notes { .. }
                | MemoryCommand::Forget { .. }
                | MemoryCommand::Export { .. }
                | MemoryCommand::Candidates { .. }
                | MemoryCommand::CandidateStatus { .. }
                | MemoryCommand::CandidateAbandon { .. }
        })
    );
    if memory_control && !exists {
        bail!("this project has no memory yet; start a conversation first");
    }
    let legacy_path = data.join("memory.sqlite3");
    let legacy = match std::fs::symlink_metadata(&legacy_path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("cannot inspect legacy memory"),
    };
    let migrate = legacy && !exists;
    if !purge && (writer || migrate) {
        if let Err(error) = Directory::ensure_private(&data) {
            return Err(data_directory_error(&data, error));
        }
        ensure_outside_workspace(&data, &cwd)?;
        if runtime_owner {
            Directory::ensure_private(&data.join("locks"))
                .context("project lock directory must be a private regular directory")?;
        }
    }
    let mut project_maintenance = if !purge && ((writer && !runtime_owner) || migrate) {
        Some(project_lease(&data, &cwd)?)
    } else {
        None
    };
    let mut project_startup = if runtime_owner && !migrate {
        Some(project_startup_lease(&data, &cwd)?)
    } else {
        None
    };
    if purge {
        if let Err(error) = Directory::ensure_private(&data) {
            return Err(data_directory_error(&data, error));
        }
        ensure_outside_workspace(&data, &cwd)?;
        let mut options = MemoryOptions::new(data.clone(), scope.clone());
        options.config = memory_config;
        let outcome = MemoryStore::purge(options).await?;
        crate::diagnostics::purge(&data, &scope)
            .context("project memory was removed but project diagnostics cleanup failed")?;
        println!("{}", serde_json::to_string_pretty(&outcome)?);
        return Ok(());
    }
    let mut diagnostics = if install_diagnostics && runtime_owner {
        crate::diagnostics::install(&data, &scope, cli.debug)?
    } else {
        None
    };
    if cli.debug
        && let Some(diagnostics) = &diagnostics
    {
        eprintln!(
            "{}",
            serde_json::json!({
                "debug_ring": diagnostics.directory().to_string_lossy(),
            })
        );
    }
    // A new installation can inspect configuration without creating state. An
    // existing store supplies only this project's interactive choices, never a
    // resumed transcript. Refuse tool-root storage before opening its database.
    let existing_memory = match async {
        if !exists && !legacy {
            return Ok::<Option<MemoryStore>, anyhow::Error>(None);
        }
        ensure_outside_workspace(&data, &cwd)?;
        let mut options = MemoryOptions::new(data.clone(), scope.clone());
        options.config = memory_config.clone();
        options.read_only = !writer && !migrate;
        Ok(Some(open_memory(options, &cwd, interactive).await?))
    }
    .await
    {
        Ok(memory) => memory,
        Err(error) => {
            if let Some(diagnostics) = diagnostics.take()
                && let Err(finish) = diagnostics.finish()
            {
                let _ = finish;
                return Err(
                    error.context("diagnostic cleanup also failed; diagnostics may be incomplete")
                );
            }
            return Err(error);
        }
    };
    // Keep cleanup outside every command/error return and retain the project
    // lease until the owned supervisor has reaped Dolt.
    let mut memory_to_close = existing_memory.clone();
    let mut run_delivery = None;
    let result = async {
        let preferences = if let Some(memory) = &existing_memory {
            Harness::load_preferences(memory, &cwd).await?
        } else {
            ProjectPreferences::default()
        };
        let mut config = snapshot.finalize(&preferences)?;
        match &cli.command {
            Some(Command::Sessions { command }) => {
                let Some(command) = command else {
                    let sessions = if let Some(memory) = &existing_memory {
                        Harness::list_sessions(memory, &cwd).await?
                    } else {
                        vec![]
                    };
                    println!("{}", serde_json::to_string_pretty(&sessions)?);
                    return Ok(());
                };
                let memory = existing_memory
                    .as_ref()
                    .context("this project has no memory yet; start a conversation first")?;
                if let SessionCommand::Export {
                    session,
                    format,
                    output,
                } = command
                {
                    crate::session_export::export(
                        memory,
                        session,
                        *format,
                        output.as_deref(),
                        &cwd,
                    )
                    .await?;
                    return Ok(());
                }
                let source = match command {
                    SessionCommand::Rename { session, .. }
                    | SessionCommand::Remove { session }
                    | SessionCommand::Restore { session }
                    | SessionCommand::Fork { session, .. } => memory
                        .session_catalog_record(session)
                        .await?
                        .context("session is absent from this project")?,
                    SessionCommand::Export { .. } => unreachable!("session export returned above"),
                };
                let outcome = match command {
                    SessionCommand::Rename { label, .. } => {
                        memory
                            .rename_session(
                                &source.session_id,
                                source.lifecycle_generation,
                                label,
                            )
                            .await?
                    }
                    SessionCommand::Remove { .. } => {
                        memory
                            .remove_session(&source.session_id, source.lifecycle_generation)
                            .await?
                    }
                    SessionCommand::Restore { .. } => {
                        memory
                            .restore_session(&source.session_id, source.lifecycle_generation)
                            .await?
                    }
                    SessionCommand::Fork {
                        node,
                        child_id,
                        label,
                        ..
                    } => {
                        let child_id = child_id
                            .clone()
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                        memory
                            .fork_session(
                                &source.session_id,
                                source.lifecycle_generation,
                                node,
                                &child_id,
                                label,
                            )
                            .await?
                    }
                    SessionCommand::Export { .. } => unreachable!("session export returned above"),
                };
                println!("{}", serde_json::to_string_pretty(&outcome)?);
                return Ok(());
            }
            Some(Command::Memory { command }) => {
                if matches!(command, MemoryCommand::Inventory | MemoryCommand::Import { .. }) {
                    unreachable!("legacy inventory/import returned before ordinary memory open")
                }
                let memory = existing_memory
                    .as_ref()
                    .context("this project has no memory yet; start a conversation first")?;
                match command {
                    MemoryCommand::Inventory | MemoryCommand::Import { .. } => {
                        unreachable!("legacy inventory/import returned before ordinary memory open")
                    }
                    MemoryCommand::Status => {
                        println!("{}", serde_json::to_string_pretty(&memory.status().await?)?)
                    }
                    MemoryCommand::History { limit } => {
                        ensure!(
                            (1..=1000).contains(limit),
                            "memory history limit must be between 1 and 1000"
                        );
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&memory.revisions(*limit).await?)?
                        );
                    }
                    MemoryCommand::Candidates { limit, after } => {
                        ensure!(
                            (1..=commands::CANDIDATE_PAGE_LIMIT).contains(limit),
                            "memory candidate limit must be between 1 and {}",
                            commands::CANDIDATE_PAGE_LIMIT
                        );
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &memory.candidate_inventory(after.as_deref(), *limit).await?
                            )?
                        );
                    }
                    MemoryCommand::CandidateStatus { branch } => {
                        let status = memory.candidate_ref_status(branch).await?;
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&commands::candidate_status_json(&status))?
                        );
                    }
                    MemoryCommand::CandidateAbandon { branch, base, head } => {
                        if let Some(previous) = memory.recover_selected_candidate_abandon().await? {
                            anyhow::bail!(
                                "previous selected candidate abandonment resolved as {previous:?}; inspect the exact ref before another action"
                            );
                        }
                        memory.reconcile().await?;
                        let inspected = memory.candidate_ref_status(branch).await?;
                        ensure!(
                            matches!(
                                inspected.state,
                                CandidateRefState::OpenUnchanged | CandidateRefState::OpenConflict
                            ) && inspected.base.as_deref() == Some(base.as_str())
                                && inspected.head.as_deref() == Some(head.as_str()),
                            "selected candidate ref changed or its outcome is unproved"
                        );
                        match memory.abandon_candidate_ref(branch, base, head).await {
                            Ok(()) => {}
                            Err(error) if error.is::<CandidateRefRejected>() => return Err(error),
                            Err(error) => {
                                settle_selected_candidate_abandon(memory, error).await?
                            }
                        }
                        println!(
                            "{}",
                            serde_json::json!({"branch": branch, "state": "abandoned"})
                        );
                    }
                    MemoryCommand::Export { format, output } => {
                        memory_export::export(memory, *format, output.as_deref(), &cwd).await?;
                    }
                    MemoryCommand::Notes { identity, limit } => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &read_notes(memory, &cwd, config.mode, identity, *limit).await?
                            )?
                        );
                    }
                    MemoryCommand::Forget { identity, note } => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &forget_note(memory, &cwd, config.mode, identity, *note).await?
                            )?
                        );
                    }
                    MemoryCommand::Purge { .. } => {
                        unreachable!("purge returned before opening memory")
                    }
                }
                return Ok(());
            }
            Some(Command::Tool { name, args }) => {
                let host = permission_host(&data, root.clone(), &config, &snapshot, matches!(name.as_str(), "file_write" | "file_edit" | "file_delete"))?;
                let result = async {
                    let arguments = serde_json::from_str(args)?;
                    let catalog = host.catalog().await?;
                    report_mcp_statuses(catalog.mcp(), &config.ui);
                    host.execute(name, arguments).await
                }
                .await;
                let cleanup = host.shutdown().await;
                let printed = result.map(|output| println!("{output}"));
                return finish(printed, cleanup, "tool host");
            }
            Some(Command::File { command }) => {
                match command {
                    FileCommand::Undo { id } => {
                        let host = permission_host(&data, root.clone(), &config, &snapshot, true)?;
                        let outcome = host.undo_file_checkpoint(id, None).await;
                        let cleanup = host.shutdown().await;
                        let printed = outcome.and_then(|outcome| {
                            println!("{}", serde_json::to_string_pretty(&outcome)?);
                            Ok(())
                        });
                        finish(printed, cleanup, "tool host")?;
                    }
                    FileCommand::List { .. }
                    | FileCommand::Inspect { .. }
                    | FileCommand::Prune { .. } => unreachable!("inspection returned before configuration"),
                }
                return Ok(());
            }
            _ => {}
        }
        if matches!(cli.command, Some(Command::UndoDream)) {
            let memory = existing_memory
                .as_ref()
                .context("this project has no memory yet; start a conversation first")?;
            if let Some(notice) = crate::memory_notice::MemoryNotice::pending(memory.clone()).await? {
                notice.announce().await?;
            }
            kuru_runtime::undo_dream(&config, &scope, memory, cli.resume.as_deref()).await?;
            human_stdout(&config.ui, Role::Accent, "Previous membership restored.");
            return Ok(());
        }
        if matches!(cli.command, Some(Command::Models)) {
            let provider = provider(&config, &cwd, &data).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&provider.models().await?)?
            );
            return Ok(());
        }
        std::fs::create_dir_all(&data)?;
        let data = data.canonicalize()?;
        ensure!(
            !data.starts_with(&cwd),
            "memory directory must be outside the tool workspace; set --data-dir to a separate directory"
        );
        let memory = match existing_memory {
            Some(memory) => memory,
            None => {
                let mut options = MemoryOptions::new(data.clone(), scope);
                options.config = memory_config;
                open_memory(options, &cwd, interactive).await?
            }
        };
        memory_to_close = Some(memory.clone());
        // Legacy activation is complete. Retain shared maintenance exclusion
        // before session admission without excluding independent conversations.
        if runtime_owner && project_maintenance.is_some() {
            drop(project_maintenance.take());
            project_startup = Some(project_startup_lease(&data, &cwd)?);
        }
        let resume = if cli.continue_session {
            Some(Harness::continuation_session(&memory, &cwd).await?)
        } else if let Some(session_id) = cli.resume.as_deref() {
            Some(Harness::resumable_session(&memory, &cwd, session_id).await?)
        } else {
            None
        };
        let admission = Harness::admit_session(memory.clone(), &cwd, resume.as_deref()).await?;
        let setup = async {
            let provider = provider(&config, &cwd, &data).await?;
            let models = select_model(&mut config, &provider).await?;
            Ok::<_, anyhow::Error>((provider, models))
        }.await;
        let (provider, models) = match setup {
            Ok(setup) => setup,
            Err(error) => return finish(Err(error), admission.close().await, "session admission"),
        };
        let notice = crate::memory_notice::MemoryNotice::pending(memory.clone()).await?;
        if matches!(
            cli.command,
            Some(Command::Run { .. } | Command::Dream | Command::Serve { .. })
        ) && let Some(notice) = &notice
        {
            notice.announce().await?;
        }
        let prompt_gate = Arc::new(crate::instruction_gate::NestedInstructionGate::new(
            root.clone(),
            data.clone(),
            snapshot.clone(),
            cli.trust_workspace_once,
        ));
        let has_skills = snapshot.prompt_catalog().skills().next().is_some();
        let tools = permission_host(&data, root.clone(), &config, &snapshot, true)?
            .with_instruction_gate(prompt_gate.clone())
            .with_skill_gate(prompt_gate, has_skills);
        let update_notice_enabled = config.update.notice;
        let presentation_ui = config.ui.clone();
        let mut harness = Harness::with_admission_and_instructions(
            config,
            &cwd,
            snapshot.instructions().to_owned(),
            admission,
            provider,
            tools,
        )
        .await?;
        match cli.command {
            Some(Command::Run {
                prompt,
                json,
                output_format,
                turn_id,
            }) => {
                let mut events = harness.subscribe();
                let turn_id = turn_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let mut delivery = crate::headless::drive(
                    &mut harness,
                    prompt.as_deref().expect("Run input was validated before authority"),
                    &turn_id,
                    json,
                    output_format,
                    invocation_signal.take().expect("Run retains its input-registered signal"),
                ).await;
                let succeeded = delivery.as_ref().is_ok_and(|delivery| delivery.result.is_ok());
                let cleanup = match &mut delivery {
                    Ok(delivery) => delivery.shutdown(&mut harness, succeeded).await,
                    Err(_) => harness.shutdown(false).await,
                };
                for event in harness.take_compaction_notices() {
                    if let Event::Compaction { actor, notice } = event {
                        // Context maintenance never changes stdout's answer/JSON.
                        let _ = writeln!(io::stderr().lock(), "{}", notice.text(&actor));
                    }
                }
                if succeeded {
                    let mut reports = std::collections::BTreeSet::new();
                    loop {
                        let event = match events.try_recv() {
                            Ok(event) => event,
                            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::TryRecvError::Empty
                            | tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                        };
                        let report = match event {
                            Event::Hook { observation, .. }
                                if observation.event == "post_turn" =>
                            {
                                Some(format!(
                                    "post-turn hook {}: {}",
                                    observation.hook_index, observation.outcome
                                ))
                            }
                            Event::Error { detail, .. }
                                if detail == HOOK_ANNOTATION_UNRESOLVED_AFTER_ANSWER =>
                            {
                                Some(detail)
                            }
                            _ => None,
                        };
                        if let Some(report) = report
                            && reports.len() < 8
                        {
                            reports.insert(report);
                        }
                    }
                    for report in reports {
                        // A closed stderr cannot undo a durably completed turn.
                        let _ = writeln!(io::stderr().lock(), "{report}");
                    }
                }
                let mut delivery = finish(delivery, cleanup, "harness setup")?;
                delivery.result = delivery.result.map_err(|error| {
                    let mut failures = std::collections::BTreeSet::new();
                    while let Ok(event) = events.try_recv() {
                        if let Event::Error { detail, .. } = event
                            && failures.len() < 8
                        {
                            failures.insert(detail.chars().take(512).collect::<String>());
                        }
                    }
                    if failures.is_empty() {
                        error
                    } else {
                        error.context(format!("provider errors: {}", failures.into_iter().collect::<Vec<_>>().join("; ")))
                    }
                });
                run_delivery = Some(delivery);
            }

            Some(Command::Dream) => {
                let result = harness.dream().await;
                let cleanup = harness.shutdown(false).await;
                let printed = result.and_then(|outcome| {
                    println!("{}", serde_json::to_string_pretty(&outcome)?);
                    Ok(())
                });
                finish(printed, cleanup, "harness")?;
            }
            Some(Command::UndoDream | Command::File { .. }) => unreachable!("local control returned before provider construction"),
            Some(Command::Serve { bind, token_env }) => {
                ensure!(
                    bind.ip().is_loopback(),
                    "v1 A2A listener must bind to loopback; put an authenticated gateway in front for remote access"
                );
                let token = std::env::var(&token_env).with_context(|| {
                    format!("set {token_env} to a bearer token of at least 16 characters")
                })?;
                let harness = Arc::new(Mutex::new(harness));
                let listener = tokio::net::TcpListener::bind(bind).await?;
                let actual = listener.local_addr()?;
                let app =
                    kuru_runtime::server::router(harness.clone(), &format!("http://{actual}"), &token)?;
                human_stderr(&presentation_ui, Role::Info, &format!("Kuru A2A listening on {actual}"));
                kuru_runtime::server::serve(listener, app).await?;
                harness.lock().await.shutdown(false).await?;
            }
            None => {
                let registry = crate::commands::Registry::from_catalog(snapshot.prompt_catalog());
                let config_projection = snapshot.display_projection(
                    &preferences,
                    ConfigDisplayBounds {
                        max_layers: 32,
                        max_rows: 64,
                        max_value_bytes: 384,
                        max_total_bytes: 48 * 1024,
                    },
                )?;
                let update_notice = if update_notice_enabled
                    && io::stdin().is_terminal()
                    && io::stdout().is_terminal()
                    && io::stderr().is_terminal()
                {
                    kuru_delivery::archive::host_target().ok().map(|target| {
                        kuru_delivery::notice::Session::start(
                            data.clone(),
                            env!("CARGO_PKG_VERSION").to_owned(),
                            target.to_owned(),
                        )
                    })
                } else {
                    None
                };
                let interactive = crate::ui::run_with_notice_commands_and_config(
                    harness,
                    models,
                    notice,
                    registry,
                    Some(config_projection),
                )
                .await;
                // This function returns only after owned terminal restoration.
                // Advice has no authority to delay exit or replace a TUI error.
                let advice = update_notice.and_then(kuru_delivery::notice::Session::finish);
                if interactive.is_ok()
                    && let Some(advice) = advice
                {
                    let _ = writeln!(io::stderr().lock(), "{advice}");
                }
                interactive?
            }
            Some(Command::Mcp { .. }) => unreachable!("MCP command returned before memory setup"),
            _ => unreachable!("early-return commands handled above"),
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let cleanup = if let Some(memory) = memory_to_close {
        memory.close().await
    } else {
        Ok(())
    };
    let diagnostic_cleanup = match diagnostics {
        Some(diagnostics) => diagnostics.finish(),
        None => Ok(()),
    };
    let settled = match (
        finish(result, cleanup, "project memory"),
        diagnostic_cleanup,
    ) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(_)) => {
            eprintln!("diagnostic cleanup failed; diagnostics may be incomplete");
            Ok(())
        }
        (Err(error), Err(_)) => {
            Err(error.context("diagnostic cleanup also failed; diagnostics may be incomplete"))
        }
        (Err(error), Ok(())) => Err(error),
    };
    // The stdout-only worker cannot extend the project writer lifetime.
    drop(project_startup);
    drop(project_maintenance);
    if let Some(delivery) = run_delivery {
        delivery.finish(settled).await
    } else {
        settled
    }
}

/// Retain the first-polled Ctrl-C listener and settle one already-owned memory
/// operation before returning. The callback can request cooperative cancel for
/// cancellable operations; explicit import intentionally settles to its receipt
/// or an honest unconfirmed error.
async fn finish_memory_operation<T>(
    operation: impl Future<Output = Result<T>>,
    on_interrupt: impl FnOnce(),
    signal: &mut Option<crate::headless::RunSignal>,
) -> Result<T> {
    let interrupt = signal.get_or_insert_with(|| Box::pin(ctrl_c_cancellation()));
    if let std::task::Poll::Ready(result) = futures::poll!(&mut *interrupt) {
        result.context("listen for memory operation cancellation")?;
        bail!("memory operation cancelled before it started");
    }
    tokio::pin!(operation);
    tokio::select! {
        biased;
        result = &mut operation => result,
        _signal_result = &mut *interrupt => {
            on_interrupt();
            let outcome = operation.await;
            match outcome {
                Ok(value) => Ok(value),
                Err(error) => Err(error.context(
                    "memory operation interrupted; owned settlement awaited; effects may already be durable",
                )),
            }
        }
    }
}

/// Combine an operation's result with the cleanup that ran after it. A
/// single failure is returned unchanged; when both fail, neither cause is
/// dropped, in the shape `Harness::shutdown` uses (primary first).
pub(crate) fn finish<T>(primary: Result<T>, cleanup: Result<()>, what: &str) -> Result<T> {
    match (primary, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => {
            let failures = [
                format!("{primary:#}"),
                format!("{what} cleanup failed: {cleanup:#}"),
            ];
            bail!(failures.join("; "))
        }
    }
}

async fn run_mcp_command(host: &ToolHost, command: &McpCommand, ui: &UiConfig) -> Result<()> {
    match command {
        McpCommand::Status { alias } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&host.mcp_oauth_status(alias).await?)?
            );
        }
        McpCommand::Logout { alias } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&host.mcp_oauth_logout(alias).await?)?
            );
        }
        McpCommand::Login {
            alias,
            device: true,
            ..
        } => {
            let login = host.begin_mcp_oauth_device(alias).await?;
            println!("Open {}", login.verification_url());
            println!("Enter code: {}", login.user_code());
            login
                .finish_with_cancellation(ctrl_c_cancellation())
                .await?;
            human_stdout(ui, Role::Accent, &format!("Signed in to MCP {alias}."));
        }
        McpCommand::Login {
            alias, no_browser, ..
        } => {
            let login = host.begin_mcp_oauth_browser(alias).await?;
            println!("Sign in to MCP {alias}:\n{}", login.authorization_url());
            if *no_browser {
                println!("{}", login.callback_guidance());
            } else if crate::authentication::open_browser(login.authorization_url())
                .await
                .is_err()
            {
                eprintln!("Open the URL above in your browser to continue.");
            }
            login
                .finish_with_cancellation(ctrl_c_cancellation())
                .await?;
            human_stdout(ui, Role::Accent, &format!("Signed in to MCP {alias}."));
        }
    }
    Ok(())
}

pub(crate) async fn ctrl_c_cancellation() -> Result<()> {
    tokio::signal::ctrl_c()
        .await
        .context("listen for Ctrl-C cancellation")
}

/// Binary-only exit classification, after execute has awaited its cleanup.
pub fn headless_exit_code(error: &anyhow::Error) -> Option<i32> {
    error
        .downcast_ref::<crate::headless::RunExit>()
        .map(|exit| exit.0)
}

fn report_mcp_statuses(statuses: &[McpStatus], ui: &UiConfig) {
    for status in statuses {
        if status.available() && status.diagnostic().is_none() {
            continue;
        }
        let state = match status.availability() {
            McpAvailability::Disabled => "disabled",
            McpAvailability::Live => "live; cache update unavailable",
            McpAvailability::Stale => "stale cached metadata; server unavailable",
            McpAvailability::Degraded => "configured server unavailable",
        };
        human_stderr(
            ui,
            Role::Warning,
            &format!("MCP {}: {state}", status.alias()),
        );
        if let Some(diagnostic) = status.diagnostic() {
            eprintln!("{diagnostic}");
        }
    }
}

fn permission_host(
    data: &Path,
    root: Arc<Directory>,
    config: &Config,
    snapshot: &ConfigSnapshot,
    checkpoints: bool,
) -> Result<ToolHost> {
    let binding = PermissionBinding::checked(&root, snapshot.manifest().full_digest(), config)?;
    let store = Arc::new(GrantStore::new(data, root.clone(), binding.clone())?);
    let permissions = Arc::new(PermissionService::new(config.clone(), binding, store)?);
    let host = ToolHost::with_permission_service(root.clone(), config, permissions)?
        .with_mcp_catalog_store(Arc::new(McpCatalogStore::new(
            data,
            root.clone(),
            snapshot.manifest().full_digest(),
        )?))?
        .with_mcp_credential_store(Arc::new(McpCredentialStore::new(
            data,
            root.clone(),
            snapshot.manifest().full_digest(),
        )?))?;
    if checkpoints {
        host.with_checkpoint_store(Arc::new(CheckpointStore::new(data, root)?))
    } else {
        Ok(host)
    }
}

pub(crate) fn all_claim_categories() -> std::collections::BTreeSet<AuthorityClaimCategory> {
    use AuthorityClaimCategory as Category;
    [
        Category::WorkspaceWrite,
        Category::Shell,
        Category::McpStdio,
        Category::McpHttp,
        Category::MemoryDoltBinary,
        Category::MemoryCacheDir,
        Category::ResponsesRoute,
        Category::ExternalAgent,
        Category::ProjectInstructions,
        Category::ProjectSkillMetadata,
        Category::ProjectSkillMaterial,
        Category::ProjectCommands,
        Category::ToolPermissions,
        Category::LifecycleHooks,
    ]
    .into_iter()
    .collect()
}

fn command_claim_categories(
    command: Option<&Command>,
) -> std::collections::BTreeSet<AuthorityClaimCategory> {
    use AuthorityClaimCategory as Category;
    let categories: &[Category] = match command {
        Some(Command::Auth) => &[Category::ResponsesRoute],
        Some(Command::Sessions { .. } | Command::Memory { .. } | Command::UndoDream) => {
            &[Category::MemoryDoltBinary, Category::MemoryCacheDir]
        }
        Some(Command::File {
            command: FileCommand::Undo { .. },
        }) => &[Category::WorkspaceWrite, Category::ToolPermissions],
        Some(Command::File { .. }) => &[],
        Some(Command::Models) => &[
            Category::MemoryDoltBinary,
            Category::MemoryCacheDir,
            Category::ResponsesRoute,
        ],
        Some(Command::Tools) => &[
            Category::WorkspaceWrite,
            Category::Shell,
            Category::ToolPermissions,
            Category::McpStdio,
            Category::McpHttp,
        ],
        Some(Command::Mcp { .. }) => &[Category::McpHttp],
        Some(Command::Tool { .. }) => &[
            Category::WorkspaceWrite,
            Category::Shell,
            Category::ToolPermissions,
            Category::McpStdio,
            Category::McpHttp,
            Category::MemoryDoltBinary,
            Category::MemoryCacheDir,
        ],
        None | Some(Command::Run { .. } | Command::Dream | Command::Serve { .. }) => &[
            Category::WorkspaceWrite,
            Category::Shell,
            Category::ToolPermissions,
            Category::McpStdio,
            Category::McpHttp,
            Category::MemoryDoltBinary,
            Category::MemoryCacheDir,
            Category::ResponsesRoute,
            Category::ExternalAgent,
            Category::ProjectInstructions,
            Category::ProjectSkillMetadata,
            Category::ProjectCommands,
            Category::LifecycleHooks,
        ],
        Some(
            Command::Completions { .. }
            | Command::Man
            | Command::Login { .. }
            | Command::Logout
            | Command::Canary { .. }
            | Command::Doctor { .. }
            | Command::Config
            | Command::Update { .. }
            | Command::Trust { .. },
        ) => &[],
    };
    categories.iter().copied().collect()
}

fn explicitly_selected_models(cli: &Cli) -> bool {
    matches!(cli.command, Some(Command::Models)) && cli.provider.is_some() && cli.model.is_some()
}

fn preflight(cli: &Cli, root: &Directory, data: &Path, snapshot: &ConfigSnapshot) -> Result<()> {
    let categories = if explicitly_selected_models(cli) {
        [AuthorityClaimCategory::ResponsesRoute]
            .into_iter()
            .collect()
    } else {
        command_claim_categories(cli.command.as_ref())
    };
    let applicable = snapshot.manifest().filtered(&categories);
    if applicable.claims().is_empty() {
        return Ok(());
    }
    let store = ApprovalStore::new(data, root);
    if store.inspect(snapshot.manifest()) == ApprovalState::Matching || cli.trust_workspace_once {
        return Ok(());
    }
    if cli.command.is_none() {
        let complete = snapshot.manifest().filtered(&all_claim_categories());
        show_manifest(root, &complete, Some(store.inspect(snapshot.manifest())));
        eprintln!(
            "This launch needs {} of the {} reviewed authority claims shown above.",
            applicable.claims().len(),
            complete.claims().len()
        );
        eprintln!("[1] Continue once  [2] Approve this complete configuration  [3] Cancel");
        eprint!("Choice [3]: ");
        io::stderr().flush()?;
        let mut choice = String::new();
        if io::stdin().read_line(&mut choice)? == 0 {
            bail!("workspace trust was not granted");
        }
        return match choice.trim() {
            "1" => Ok(()),
            "2" => {
                root.revalidate()
                    .context("workspace changed before approval was recorded")?;
                store.approve_tui(snapshot.manifest())
            }
            _ => bail!("workspace trust was not granted"),
        };
    }
    let refusal = format!(
        "workspace authority is not approved for this command\n{}\nRun `kuru trust approve` with the same `-C` directory, or repeat this command with `--trust-workspace-once` after review.",
        manifest_text(root, &applicable, Some(store.inspect(snapshot.manifest()))),
    );
    if matches!(cli.command, Some(Command::Run { .. })) {
        Err(anyhow::Error::new(crate::headless::RunExit(3)).context(refusal))
    } else {
        Err(anyhow::anyhow!(refusal))
    }
}

fn show_manifest(root: &Directory, manifest: &SafeManifest, state: Option<ApprovalState>) {
    println!("{}", manifest_text(root, manifest, state));
}

pub(crate) fn manifest_text(
    root: &Directory,
    manifest: &SafeManifest,
    state: Option<ApprovalState>,
) -> String {
    let mut text = format!(
        "Workspace: {}\nManifest: v{} {}",
        safe_path(root.path()),
        manifest.schema_version(),
        manifest.digest()
    );
    if let Some(state) = state {
        let label = match state {
            ApprovalState::Absent => "not approved".to_owned(),
            ApprovalState::Matching => "approved".to_owned(),
            ApprovalState::Stale => "approval does not match the current manifest".to_owned(),
            ApprovalState::Invalid => "approval state is invalid or unsafe".to_owned(),
            ApprovalState::Unreadable(failure) => format!(
                "approval record could not be read ({failure}); fix the permissions of the approval record or the Kuru data directory, then retry"
            ),
        };
        text.push_str(&format!("\nStatus: {label}"));
    }
    if manifest.sources().is_empty() {
        text.push_str("\nAutomatic project instruction sources: none");
    } else {
        text.push_str("\nAutomatic project instruction sources:");
        for source in manifest.sources() {
            text.push_str(&format!("\n  - {source}"));
        }
    }
    if manifest.claims().is_empty() {
        text.push_str("\nAuthority claims: none");
    } else {
        text.push_str("\nAuthority claims:");
        for claim in manifest.claims() {
            if claim.category() == AuthorityClaimCategory::ProjectInstructions {
                text.push_str(&format!(
                    "\n  - {}: {}",
                    claim.category().label(),
                    claim.display()
                ));
                for (index, source) in claim.sources().iter().enumerate() {
                    text.push_str(&format!("\n      {}. {source}", index + 1));
                }
            } else {
                text.push_str(&format!(
                    "\n  - {}: {} (source {})",
                    claim.category().label(),
                    claim.display(),
                    claim.source()
                ));
            }
        }
    }
    text
}

fn confirm(prompt: &str) -> Result<bool> {
    ensure!(
        io::stdin().is_terminal() && io::stderr().is_terminal(),
        "confirmation requires a terminal; use --yes after reviewing the manifest"
    );
    eprint!("{prompt}");
    io::stderr().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Ok(false);
    }
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn safe_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    let mut safe = String::new();
    for character in value.chars() {
        if safe.len() >= 512 {
            safe.push('…');
            break;
        }
        safe.extend(character.escape_default());
    }
    safe
}

/// Advisory OS locks release when the process exits, including crashes. Keep the
/// file itself: unlinking lockfiles would allow competing locks on new inodes.
struct ProjectLease {
    _file: File,
    _directory: Directory,
}

fn ensure_outside_workspace(data: &Path, workspace: &Path) -> Result<()> {
    let data = Directory::open(data, Privacy::Inherited, NameRetention::Movable)?;
    let workspace = Directory::open(workspace, Privacy::Inherited, NameRetention::Movable)?;
    ensure!(
        !data.is_within(&workspace)?,
        "memory directory must be outside the tool workspace; set --data-dir to a separate directory"
    );
    Ok(())
}

fn project_lease(data: &Path, cwd: &Path) -> Result<ProjectLease> {
    checked_project_lease(data, cwd, false)
}

fn project_startup_lease(data: &Path, cwd: &Path) -> Result<ProjectLease> {
    checked_project_lease(data, cwd, true)
}

fn checked_project_lease(data: &Path, cwd: &Path, shared: bool) -> Result<ProjectLease> {
    let directory = Directory::ensure_private(&data.join("locks"))
        .context("project lock directory must be a private regular directory")?;
    let digest = Sha256::digest(cwd.as_os_str().as_encoded_bytes());
    let name = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let name = std::ffi::OsString::from(format!("{name}.lock"));
    let file = directory
        .lock_file(&name)
        .context("cannot open the project writer lock")?;
    if shared {
        file.try_lock_shared().map_err(|error| {
            anyhow::anyhow!("project maintenance ownership is unavailable: {error}")
        })?;
    } else {
        file.try_lock().map_err(|error| {
            anyhow::anyhow!(
                "project already has an active Kuru writer, or its lock could not be acquired: {error}"
            )
        })?;
    }
    directory.verify(&name, &file)?;
    Ok(ProjectLease {
        _file: file,
        _directory: directory,
    })
}

async fn update(
    version: Option<&str>,
    release_base: Option<&str>,
    source: Option<&Path>,
) -> Result<()> {
    if source.is_some() {
        ensure!(
            version.is_none() && release_base.is_none(),
            "--source cannot be combined with release options"
        );
    } else {
        version.context("provide --version VERSION for a verified release or --source CHECKOUT")?;
    }
    #[cfg(unix)]
    return update_unix(version, release_base, source).await;
    #[cfg(windows)]
    {
        if let Some(source) = source {
            let candidate = build_windows_source(source).await?;
            let outcome =
                kuru_delivery::update::replace_running_binary(&candidate, &update_helper_cache()?)
                    .await?;
            println!("Installed source build at {}", outcome.installed.display());
        } else {
            let version = version
                .context("provide --version VERSION for a verified release or --source CHECKOUT")?;
            let configured_base = std::env::var("KURU_RELEASE_BASE").ok();
            let base = release_base
                .or(configured_base.as_deref())
                .context("--release-base or KURU_RELEASE_BASE is required")?;
            let path =
                kuru_delivery::update::replace_running(base, version, &update_helper_cache()?)
                    .await?
                    .installed;
            println!(
                "Installed Kuru {} at {}",
                kuru_delivery::archive::checked_version(version)?,
                path.display()
            );
        }
        Ok(())
    }
}

#[cfg(unix)]
async fn recover_unix_installation(
    existing_signal: &mut Option<crate::headless::RunSignal>,
) -> Result<()> {
    let Ok(executable) = std::env::current_exe() else {
        return Ok(());
    };
    let Some(parent) = executable.parent().map(Path::to_path_buf) else {
        return Ok(());
    };
    match kuru_delivery::unix_update::has_pending(&parent) {
        Ok(false) => return Ok(()),
        Err(_) => {
            eprintln!(
                "Pending Kuru installation state could not be inspected safely; state retained."
            );
            return Ok(());
        }
        Ok(true) => {}
    }
    let signal = existing_signal.get_or_insert_with(|| Box::pin(ctrl_c_cancellation()));
    // A pending transaction can mutate the installed image. Register before
    // that worker starts. Retain it through dispatch so a queued signal cannot
    // disappear between recovery and a later owned memory operation.
    if let std::task::Poll::Ready(result) = futures::poll!(&mut *signal) {
        result?;
        return Err(crate::headless::RunExit(130).into());
    }
    let mut recovery =
        tokio::task::spawn_blocking(move || kuru_delivery::unix_update::recover(&parent));
    let mut interrupted = None;
    let result = tokio::select! {
        biased;
        result = &mut recovery => result,
        result = &mut *signal => {
            interrupted = Some(result);
            recovery.await
        }
    };
    match result {
        Ok(Ok(None)) => {}
        Ok(Err(error))
            if error
                .downcast_ref::<kuru_delivery::unix_update::Busy>()
                .is_some() => {}
        Ok(Ok(Some(outcome))) => {
            eprintln!(
                "Pending Kuru installation {}.",
                if outcome.published {
                    "finished forward"
                } else {
                    "settled without replacement"
                }
            );
            if outcome.support_incomplete {
                eprintln!("Executable installed; stable man page publication is incomplete.");
            }
        }
        _ => eprintln!(
            "Pending Kuru installation recovery refused; state retained for a compatible updater."
        ),
    }
    if let Some(signal) = interrupted {
        signal?;
        return Err(crate::headless::RunExit(130).into());
    }
    Ok(())
}

#[cfg(unix)]
async fn update_unix(
    version: Option<&str>,
    release_base: Option<&str>,
    source: Option<&Path>,
) -> Result<()> {
    // Install this one listener before any owned build can launch. Dropping a
    // future alone cannot clean up after the operating system's default SIGINT.
    let mut signal: crate::headless::RunSignal = Box::pin(ctrl_c_cancellation());
    if let std::task::Poll::Ready(result) = futures::poll!(&mut signal) {
        result?;
        return Err(crate::headless::RunExit(130).into());
    }
    let executable = std::env::current_exe()?;
    let environment = kuru_delivery::ownership::OwnershipEnv::capture();
    // Manager refusal and permission preflight precede even recovery state.
    let mut installation = kuru_delivery::ownership::installed(&executable, &environment)?;
    let destination = executable
        .parent()
        .context("executable has no parent")?
        .to_path_buf();
    let parent = destination.clone();
    let mut recovery =
        tokio::task::spawn_blocking(move || kuru_delivery::unix_update::recover(&parent));
    let recovered = tokio::select! {
        biased;
        result = &mut recovery => result.context("installation recovery worker failed")??,
        result = &mut signal => {
            let settled = recovery.await.context("installation recovery worker failed")?;
            settled?;
            result?;
            return Err(crate::headless::RunExit(130).into());
        }
    };
    if recovered.is_some() {
        installation = kuru_delivery::ownership::installed(&executable, &environment)?;
    }
    let host = kuru_delivery::archive::host_target()?.to_owned();
    let (candidate, support) = if let Some(source) = source {
        if let std::task::Poll::Ready(result) = futures::poll!(&mut signal) {
            result?;
            return Err(crate::headless::RunExit(130).into());
        }
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let build = kuru_delivery::unix_source::build(source, cancelled);
        tokio::pin!(build);
        let path = tokio::select! {
            biased;
            result = &mut build => result?,
            result = &mut signal => {
                let _ = cancel.send(());
                if let Err(error) = build.await
                    && error.downcast_ref::<kuru_delivery::unix_source::Cancelled>().is_none() {
                    return Err(error);
                }
                result?;
                return Err(crate::headless::RunExit(130).into());
            }
        };
        (
            kuru_delivery::unix_update::Candidate::BuildInput(path),
            None,
        )
    } else {
        let version = version.context("verified release version is required")?;
        kuru_delivery::archive::checked_version(version)?;
        let configured_base = std::env::var("KURU_RELEASE_BASE").ok();
        let base = release_base
            .or(configured_base.as_deref())
            .context("--release-base or KURU_RELEASE_BASE is required")?;
        let (bytes, support) = tokio::select! {
            biased;
            result = kuru_delivery::archive::verified_release(base, version, &host) => result?,
            result = &mut signal => {
                result?;
                return Err(crate::headless::RunExit(130).into());
            }
        };
        (kuru_delivery::unix_update::Candidate::Bytes(bytes), support)
    };
    if let std::task::Poll::Ready(result) = futures::poll!(&mut signal) {
        result?;
        return Err(crate::headless::RunExit(130).into());
    }
    let request = kuru_delivery::unix_update::Request {
        installation,
        candidate,
        version: version.map(str::to_owned),
        target: host,
        support,
    };
    let mut transaction =
        tokio::task::spawn_blocking(move || kuru_delivery::unix_update::transact(request));
    let outcome = tokio::select! {
        biased;
        result = &mut transaction => result.context("update transaction worker failed")??,
        result = &mut signal => {
            // A publication may already have occurred. Await its same durable
            // settlement/error rather than dropping a live filesystem writer.
            transaction.await.context("update transaction worker failed")??;
            result?;
            return Err(crate::headless::RunExit(130).into());
        }
    };
    if outcome.support_incomplete {
        anyhow::bail!("executable installed, but stable man page publication is incomplete");
    }
    println!(
        "Installed Kuru{} at {:?}",
        version
            .map(|version| format!(" {version}"))
            .unwrap_or_else(|| " source build".into()),
        outcome.installed
    );
    Ok(())
}

#[cfg(windows)]
fn update_helper_cache() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(native_data_directory)
        .context("set LOCALAPPDATA or XDG_CACHE_HOME for update recovery storage")?;
    let path = base.join("kuru/update-helpers");
    ensure!(
        path.is_absolute(),
        "update recovery cache must be an absolute path"
    );
    Ok(path)
}

#[cfg(windows)]
async fn build_windows_source(source: &Path) -> Result<PathBuf> {
    use kuru_delivery::command::{output, rooted};
    use std::time::Duration;
    let source = source
        .canonicalize()
        .context("source checkout does not exist")?;
    let host = kuru_delivery::archive::host_target()?;
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
        let mut command = rooted(&source, "mise");
        command
            .arg("-C")
            .arg(&source)
            .args(arguments)
            .current_dir(&source)
            .env("MISE_NO_HOOKS", "1")
            .env("MISE_TASK_RUN_AUTO_INSTALL", "false")
            // The mr-boxington build cache is a maintainer tool.
            .env("KURU_MBX", "0")
            .env_remove("CARGO_BUILD_TARGET");
        let result = output(&mut command, Duration::from_secs(1800))
            .await
            .context("run source build through mise")?;
        print!("{}", String::from_utf8_lossy(&result.stdout));
        eprint!("{}", String::from_utf8_lossy(&result.stderr));
        ensure!(
            result.status.success(),
            "source build failed before update publication"
        );
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| source.join("target"));
    let target = if target.is_absolute() {
        target
    } else {
        source.join(target)
    };
    Ok(target.join(host).join("release/kuru.exe"))
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn tools_inspection_activates_tool_authority_without_unrelated_memory_claims() {
        let command = Command::Tools;
        let claims = command_claim_categories(Some(&command));
        assert!(claims.contains(&AuthorityClaimCategory::ToolPermissions));
        assert!(claims.contains(&AuthorityClaimCategory::McpStdio));
        assert!(claims.contains(&AuthorityClaimCategory::McpHttp));
        assert!(!claims.contains(&AuthorityClaimCategory::MemoryDoltBinary));
        assert!(!claims.contains(&AuthorityClaimCategory::MemoryCacheDir));
        assert!(!claims.contains(&AuthorityClaimCategory::ResponsesRoute));
    }

    #[test]
    fn mcp_commands_parse_and_activate_only_http_mcp_authority() {
        for arguments in [
            vec!["kuru", "mcp", "login", "server"],
            vec!["kuru", "mcp", "login", "server", "--device"],
            vec!["kuru", "mcp", "login", "server", "--no-browser"],
            vec!["kuru", "mcp", "status", "server"],
            vec!["kuru", "mcp", "logout", "server"],
        ] {
            let cli = Cli::try_parse_from(arguments).unwrap();
            let claims = command_claim_categories(cli.command.as_ref());
            assert_eq!(
                claims,
                [AuthorityClaimCategory::McpHttp].into_iter().collect()
            );
        }
        assert!(
            Cli::try_parse_from(["kuru", "mcp", "login", "server", "--device", "--no-browser"])
                .is_err()
        );
    }

    #[tokio::test]
    async fn automatic_permission_claim_is_reviewed_but_trust_does_not_grant_a_call() {
        kuru_memory::test_support::closing(async {
            let _gate = crate::spawn_gate::locking_async().await;
            let temporary = tempfile::tempdir().unwrap();
            let project = temporary.path().join("project");
            let data = temporary.path().join("data");
            std::fs::create_dir_all(project.join(".kuru")).unwrap();
            std::fs::write(
                project.join(".kuru/config.toml"),
                concat!(
                    "[[permissions]]\naction='ask'\n",
                    "selector={kind='native',name='file_write'}\n"
                ),
            )
            .unwrap();
            let root = Arc::new(
                Directory::open(&project, Privacy::Inherited, NameRetention::Movable).unwrap(),
            );
            let snapshot =
                ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default())
                    .unwrap();
            let cli = Cli::try_parse_from(["kuru", "tool", "file_write", "--args", "{}"]).unwrap();
            let error = preflight(&cli, &root, &data, &snapshot).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("workspace authority is not approved")
            );
            assert!(all_claim_categories().contains(&AuthorityClaimCategory::ToolPermissions));
            ApprovalStore::new(&data, &root)
                .approve_command(snapshot.manifest())
                .unwrap();
            preflight(&cli, &root, &data, &snapshot).unwrap();
            let config = snapshot.finalize(&ProjectPreferences::default()).unwrap();
            let host = permission_host(&data, root, &config, &snapshot, true).unwrap();
            let refused = host
                .execute(
                    "file_write",
                    serde_json::json!({"path":"note.txt","content":"must not be written"}),
                )
                .await
                .unwrap_err();
            assert!(
                kuru_connectors::is_permission_denied(&refused),
                "unexpected projected tool error: {refused:#}"
            );
            assert!(!project.join("note.txt").exists());
            host.shutdown().await.unwrap();
        })
        .await
    }
}

#[cfg(test)]
mod candidate_command_tests {
    use super::*;

    #[test]
    fn exact_candidate_commands_parse_without_a_promotion_surface() {
        let candidates = Cli::try_parse_from([
            "kuru",
            "memory",
            "candidates",
            "--limit",
            "7",
            "--after",
            "opaque",
        ])
        .unwrap();
        assert!(matches!(
            candidates.command,
            Some(Command::Memory {
                command: MemoryCommand::Candidates {
                    limit: 7,
                    after: Some(ref cursor)
                }
            }) if cursor == "opaque"
        ));
        let candidates = Cli::try_parse_from(["kuru", "memory", "candidates"]).unwrap();
        assert!(matches!(
            candidates.command,
            Some(Command::Memory {
                command: MemoryCommand::Candidates {
                    limit: commands::CANDIDATE_PAGE_LIMIT,
                    after: None
                }
            })
        ));
        let abandon = Cli::try_parse_from([
            "kuru",
            "memory",
            "candidate-abandon",
            "branch",
            "--base",
            "base",
            "--head",
            "head",
        ])
        .unwrap();
        assert!(matches!(
            abandon.command,
            Some(Command::Memory {
                command: MemoryCommand::CandidateAbandon {
                    branch,
                    base,
                    head
                }
            }) if branch == "branch" && base == "base" && head == "head"
        ));
        assert!(Cli::try_parse_from(["kuru", "memory", "candidate-promote", "branch"]).is_err());
    }
}

#[cfg(test)]
mod cleanup_combination_tests {
    use super::*;

    #[tokio::test]
    async fn retained_startup_signal_refuses_memory_before_admission() {
        kuru_memory::test_support::closing(async {
            let (send, receive) = tokio::sync::oneshot::channel();
            let mut signal: Option<crate::headless::RunSignal> = Some(Box::pin(async move {
                receive.await.context("retained fixture signal")
            }));
            // Startup recovery registered this receiver before returning. A
            // signal queued during subsequent configuration remains observable.
            assert!(futures::poll!(signal.as_mut().unwrap()).is_pending());
            send.send(()).unwrap();
            let admitted = std::cell::Cell::new(false);
            let operation = async {
                admitted.set(true);
                Ok(7)
            };
            let error = finish_memory_operation(operation, || {}, &mut signal)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "memory operation cancelled before it started"
            );
            assert!(!admitted.get());
        })
        .await;
    }

    #[tokio::test]
    async fn retained_startup_signal_awaits_accepted_memory_receipt() {
        kuru_memory::test_support::closing(async {
            let (send, receive) = tokio::sync::oneshot::channel();
            let mut signal: Option<crate::headless::RunSignal> = Some(Box::pin(async move {
                receive.await.context("retained fixture signal")
            }));
            assert!(futures::poll!(signal.as_mut().unwrap()).is_pending());
            let settle = tokio::sync::Semaphore::new(0);
            let confirmed = std::cell::Cell::new(false);
            let operation = async {
                // The operation was actually polled before the signal arrived.
                send.send(()).unwrap();
                settle.acquire().await.unwrap().forget();
                confirmed.set(true);
                Ok("confirmed import receipt")
            };
            let receipt = finish_memory_operation(operation, || settle.add_permits(1), &mut signal)
                .await
                .unwrap();
            assert!(confirmed.get());
            assert_eq!(receipt, "confirmed import receipt");
        })
        .await;
    }

    #[test]
    fn success_with_clean_cleanup_returns_the_value() {
        assert_eq!(finish(Ok(7), Ok(()), "tool host").unwrap(), 7);
    }

    #[test]
    fn cleanup_failure_after_success_is_returned_unchanged() {
        let error = finish(
            Ok(7),
            Err(anyhow::anyhow!("child not reaped").context("MCP shutdown")),
            "tool host",
        )
        .unwrap_err();
        assert_eq!(format!("{error:#}"), "MCP shutdown: child not reaped");
    }

    #[test]
    fn primary_failure_with_clean_cleanup_is_returned_unchanged() {
        let error = finish::<()>(
            Err(anyhow::anyhow!("tool refused").context("execute")),
            Ok(()),
            "tool host",
        )
        .unwrap_err();
        assert_eq!(format!("{error:#}"), "execute: tool refused");
    }

    #[test]
    fn both_failures_name_the_primary_then_the_cleanup_cause() {
        let error = finish::<()>(
            Err(anyhow::anyhow!("tool refused").context("execute")),
            Err(anyhow::anyhow!("child not reaped").context("MCP shutdown")),
            "tool host",
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert_eq!(
            message,
            "execute: tool refused; tool host cleanup failed: MCP shutdown: child not reaped"
        );
        let primary = message.find("execute: tool refused").unwrap();
        let cleanup = message.find("child not reaped").unwrap();
        assert!(primary < cleanup, "primary must precede cleanup: {message}");
    }
}
