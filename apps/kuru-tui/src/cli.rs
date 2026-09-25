use std::{
    ffi::OsStr,
    fs::File,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use kuru_connectors::permissions::{PermissionBinding, PermissionService};
use kuru_connectors::{
    CheckpointStore, McpAvailability, McpCatalogStore, McpStatus, Provider, ToolHost, provider,
};
use kuru_core::{
    AuthorityClaimCategory, Config, ConfigSnapshot, InvocationOverrides, Mode, ModelInfo,
    ProjectPreferences, SafeManifest,
};
use kuru_memory::{
    CandidateRefRejected, CandidateRefState, MemoryOpenStage, MemoryStore,
    OpenOptions as MemoryOptions, SelectedAbandonResolution, SelectedAbandonUncertain,
};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use kuru_runtime::{CancellationToken, Event, Harness, forget_note, read_notes};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{
    commands, memory_export,
    permission_store::GrantStore,
    trust::{ApprovalState, ApprovalStore},
};

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
    /// Execute a prompt without the terminal UI.
    Run {
        prompt: String,
        #[arg(long)]
        json: bool,
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
    /// Discover provider models and supported reasoning efforts.
    Models,
    /// Print merged effective configuration.
    Config,
    Sessions,
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

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
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

fn native_config_directory() -> Option<PathBuf> {
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

fn invocation_overrides(cli: &Cli) -> InvocationOverrides {
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
async fn discovered_local(root: &Directory) -> Result<Option<(PathBuf, String)>> {
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
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), command.status())
            .await
            .map_err(|_| anyhow::anyhow!("project-local Git index check timed out"))?
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

/// The real stderr sink: each write locks stderr for the call, matching the
/// prior direct-`io::stderr()` behavior exactly.
struct StderrSink;

impl Write for StderrSink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        io::stderr().lock().write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stderr().lock().flush()
    }
}

struct MemoryProgressOutput {
    sink: Box<dyn Write + Send>,
    terminal: bool,
    enabled: bool,
    width: usize,
}

impl MemoryProgressOutput {
    fn new() -> Self {
        Self::with_sink(Box::new(StderrSink), io::stderr().is_terminal())
    }

    fn with_sink(sink: Box<dyn Write + Send>, terminal: bool) -> Self {
        Self {
            sink,
            terminal,
            enabled: true,
            width: 0,
        }
    }

    /// Write one already-formatted line as a single buffered call, so the
    /// real stderr sink locks exactly once per line regardless of how many
    /// fragments the caller's format string had.
    fn write_line(&mut self, line: &str) {
        if self
            .sink
            .write_all(line.as_bytes())
            .and_then(|()| self.sink.flush())
            .is_err()
        {
            self.enabled = false;
        }
    }

    fn stage(&mut self, stage: MemoryOpenStage) {
        if !self.enabled || stage == MemoryOpenStage::Ready {
            return;
        }
        let text = memory_open_label(stage);
        let line = if self.terminal {
            let padding = " ".repeat(self.width.saturating_sub(text.len()));
            format!("\r{text}{padding}")
        } else {
            format!("{text}\n")
        };
        self.write_line(&line);
        if self.enabled {
            self.width = self.width.max(text.len());
        }
    }

    fn complete(&mut self) {
        if !self.enabled {
            return;
        }
        let text = memory_open_label(MemoryOpenStage::Ready);
        let line = if self.terminal {
            let padding = " ".repeat(self.width.saturating_sub(text.len()));
            format!("\r{text}{padding}\n")
        } else {
            format!("{text}\n")
        };
        self.write_line(&line);
    }

    fn abandon(&mut self) {
        if !self.enabled || !self.terminal || self.width == 0 {
            return;
        }
        self.write_line(&format!("\r{}\r", " ".repeat(self.width)));
    }

    /// A retained line printed after the terminal `Memory: ready.` line, never
    /// overwritten by a later transient stage. Stdout stays untouched; this is
    /// stderr only and never runs when the open failed.
    fn notice(&mut self, text: &str) {
        if !self.enabled {
            return;
        }
        self.write_line(&format!("{text}\n"));
    }
}

fn memory_open_label(stage: MemoryOpenStage) -> &'static str {
    match stage {
        MemoryOpenStage::WaitingForProjectOwnership => "Memory: waiting for project ownership…",
        MemoryOpenStage::WaitingForRuntimeCache => "Memory: waiting for verified runtime cache…",
        MemoryOpenStage::VerifyingRuntimeCache => "Memory: verifying cached runtime…",
        MemoryOpenStage::ExtractingEmbeddedRuntime => "Memory: extracting embedded runtime…",
        MemoryOpenStage::CheckingRuntimeVersion => "Memory: checking runtime version…",
        MemoryOpenStage::PreparingDatabase => "Memory: preparing database…",
        MemoryOpenStage::OpeningDatabase => "Memory: opening database…",
        MemoryOpenStage::Ready => "Memory: ready.",
        MemoryOpenStage::RetainedInstallStage => {
            "Memory: retained an install stage for later cleanup."
        }
        MemoryOpenStage::RetainedUnreceiptedInstallStage => {
            "Memory: retained an install stage, but could not record it; \
             it needs manual removal (see --debug diagnostics)."
        }
        _ => "Memory: preparing database…",
    }
}

#[cfg(test)]
mod memory_progress_output_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct BufferSink(Arc<Mutex<Vec<u8>>>);

    impl Write for BufferSink {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn captured(output: MemoryProgressOutput, buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        drop(output);
        String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn retained_install_stage_has_its_own_notice_text_distinct_from_every_other_label() {
        let text = memory_open_label(MemoryOpenStage::RetainedInstallStage);
        assert_eq!(text, "Memory: retained an install stage for later cleanup.");
        for stage in [
            MemoryOpenStage::WaitingForProjectOwnership,
            MemoryOpenStage::WaitingForRuntimeCache,
            MemoryOpenStage::VerifyingRuntimeCache,
            MemoryOpenStage::ExtractingEmbeddedRuntime,
            MemoryOpenStage::CheckingRuntimeVersion,
            MemoryOpenStage::PreparingDatabase,
            MemoryOpenStage::OpeningDatabase,
            MemoryOpenStage::Ready,
            MemoryOpenStage::RetainedUnreceiptedInstallStage,
        ] {
            assert_ne!(memory_open_label(stage), text);
        }
    }

    #[test]
    fn an_unreceipted_retained_stage_never_promises_a_later_cleanup() {
        let text = memory_open_label(MemoryOpenStage::RetainedUnreceiptedInstallStage);
        assert!(
            !text.contains("for later cleanup"),
            "an uncollectable stage must not be announced as scheduled: {text}"
        );
        assert!(text.contains("manual removal"), "{text}");
    }

    #[test]
    fn notice_is_retained_after_the_terminal_ready_line_on_one_non_terminal_stream() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let mut output =
            MemoryProgressOutput::with_sink(Box::new(BufferSink(buffer.clone())), false);
        output.stage(MemoryOpenStage::WaitingForProjectOwnership);
        output.complete();
        output.notice(memory_open_label(MemoryOpenStage::RetainedInstallStage));
        let rendered = captured(output, &buffer);
        assert_eq!(
            rendered,
            concat!(
                "Memory: waiting for project ownership…\n",
                "Memory: ready.\n",
                "Memory: retained an install stage for later cleanup.\n",
            )
        );
    }

    #[test]
    fn notice_never_writes_once_the_sink_has_failed() {
        struct FailingSink;
        impl Write for FailingSink {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("fixture sink failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = MemoryProgressOutput::with_sink(Box::new(FailingSink), false);
        output.complete();
        assert!(!output.enabled, "a failed write must disable this output");
        // Disabled output takes no further action; this only proves `notice`
        // does not panic or attempt a write once disabled.
        output.notice("must not be sent to a disabled sink");
    }
}

async fn open_memory(options: MemoryOptions, project: &Path) -> Result<MemoryStore> {
    let executable = std::env::current_exe().context("locate the current Kuru executable")?;
    let (mut progress, opening) =
        MemoryStore::open_managed_observed(options, project.to_owned(), executable);
    let mut opening = Box::pin(opening);
    let mut output = MemoryProgressOutput::new();
    let mut observed_ready = false;
    let mut retained_install_stage = None;
    let mut progress_open = true;
    let result = loop {
        tokio::select! {
            result = &mut opening => break result,
            stage = progress.recv(), if progress_open => match stage {
                Some(MemoryOpenStage::Ready) => observed_ready = true,
                Some(stage @ (MemoryOpenStage::RetainedInstallStage
                    | MemoryOpenStage::RetainedUnreceiptedInstallStage)) => {
                    retained_install_stage = Some(stage);
                }
                Some(stage) => output.stage(stage),
                None => progress_open = false,
            },
        }
    };
    drop(opening);
    while let Some(stage) = progress.recv().await {
        match stage {
            MemoryOpenStage::Ready => observed_ready = true,
            stage @ (MemoryOpenStage::RetainedInstallStage
            | MemoryOpenStage::RetainedUnreceiptedInstallStage) => {
                retained_install_stage = Some(stage);
            }
            stage => output.stage(stage),
        }
    }
    match result {
        Ok(store) => {
            // Ready is emitted only with a completed usable store. Keeping this
            // check makes a future memory stage addition unable to create a
            // synthetic success line by itself.
            debug_assert!(observed_ready, "successful observed open must report ready");
            output.complete();
            // Retained only after the terminal ready line, and only on a
            // successful open: stdout stays JSON-clean, and an abandoned open
            // never prints this notice.
            // The unreceipted case says so: nothing will collect that stage.
            if let Some(stage) = retained_install_stage {
                output.notice(memory_open_label(stage));
            }
            Ok(store)
        }
        Err(error) => {
            output.abandon();
            Err(error)
        }
    }
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

pub async fn run() -> Result<()> {
    execute(Cli::parse()).await
}

/// Binary-only entrypoint. Library callers remain subscriber-neutral.
pub async fn run_with_diagnostics() -> Result<()> {
    execute_inner(Cli::parse(), true).await
}

pub async fn execute(cli: Cli) -> Result<()> {
    execute_inner(cli, false).await
}

async fn execute_inner(cli: Cli, install_diagnostics: bool) -> Result<()> {
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

    if cli.command.is_none() && !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
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

    // Catalog inspection needs the reviewed tool/MCP authority, but it must
    // remain independent of project memory and its configured runtime. Use
    // defaults for memory-backed preferences and return before deriving a
    // memory scope, lease, or store path.
    if matches!(cli.command, Some(Command::Tools)) {
        let config = snapshot.finalize(&ProjectPreferences::default())?;
        let host = permission_host(&data, root.clone(), &config, &snapshot, false)?;
        let catalog = host.catalog().await;
        let cleanup = host.shutdown().await;
        let catalog = catalog?;
        report_mcp_statuses(catalog.mcp());
        println!("{}", serde_json::to_string_pretty(&catalog)?);
        cleanup?;
        return Ok(());
    }

    let scope = kuru_runtime::project_scope(&cwd)?;
    let memory_config = snapshot.memory_config().clone();
    let writer = matches!(
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
    let _lease = if writer || migrate {
        if let Err(error) = Directory::ensure_private(&data) {
            return Err(data_directory_error(&data, error));
        }
        ensure_outside_workspace(&data, &cwd)?;
        Some(project_lease(&data, &cwd)?)
    } else {
        None
    };
    if purge {
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
        Ok(Some(open_memory(options, &cwd).await?))
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
    let result = async {
        let preferences = if let Some(memory) = &existing_memory {
            Harness::load_preferences(memory, &cwd).await?
        } else {
            ProjectPreferences::default()
        };
        let mut config = snapshot.finalize(&preferences)?;
        match &cli.command {
            Some(Command::Sessions) => {
                let sessions = if let Some(memory) = &existing_memory {
                    Harness::list_sessions(memory, &cwd).await?
                } else {
                    vec![]
                };
                println!("{}", serde_json::to_string_pretty(&sessions)?);
                return Ok(());
            }
            Some(Command::Memory { command }) => {
                let memory = existing_memory
                    .as_ref()
                    .context("this project has no memory yet; start a conversation first")?;
                match command {
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
                    report_mcp_statuses(catalog.mcp());
                    host.execute(name, arguments).await
                }
                .await;
                let cleanup = host.shutdown().await;
                println!("{}", result?);
                cleanup?;
                return Ok(());
            }
            Some(Command::File { command }) => {
                match command {
                    FileCommand::Undo { id } => {
                        let host = permission_host(&data, root.clone(), &config, &snapshot, true)?;
                        let outcome = host.undo_file_checkpoint(id, None).await;
                        let cleanup = host.shutdown().await;
                        println!("{}", serde_json::to_string_pretty(&outcome?)?);
                        cleanup?;
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
            println!("Previous membership restored.");
            return Ok(());
        }
        let provider = provider(&config, &cwd, &data).await?;
        if matches!(cli.command, Some(Command::Models)) {
            println!(
                "{}",
                serde_json::to_string_pretty(&provider.models().await?)?
            );
            return Ok(());
        }
        let models = select_model(&mut config, &provider).await?;
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
                open_memory(options, &cwd).await?
            }
        };
        memory_to_close = Some(memory.clone());
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
        let mut harness = Harness::with_tool_host_and_instructions(
            config,
            &cwd,
            snapshot.instructions().to_owned(),
            memory,
            provider,
            cli.resume.as_deref(),
            tools,
        )
        .await?;
        match cli.command {
            Some(Command::Run {
                prompt,
                json,
                turn_id,
            }) => {
                let mut events = harness.subscribe();
                let turn_id = turn_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let cancellation = CancellationToken::new();
                let result = harness
                    .run_local_controlled(&prompt, None, &turn_id, &cancellation)
                    .await
                    .map(|result| result.output);
                let succeeded = result.is_ok();
                if let Ok(result) = &result {
                    if json {
                        println!("{}", serde_json::to_string_pretty(result)?);
                    } else {
                        println!("{}", result.text);
                    }
                }
                let cleanup = harness.shutdown(succeeded).await;
                result.map_err(|error| {
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
                })?;
                cleanup?;
            }

            Some(Command::Dream) => {
                let result = harness.dream().await;
                let cleanup = harness.shutdown(false).await;
                println!("{}", serde_json::to_string_pretty(&result?)?);
                cleanup?;
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
                eprintln!("Kuru A2A listening on {actual}");
                kuru_runtime::server::serve(listener, app).await?;
                harness.lock().await.shutdown(false).await?;
            }
            None => {
                let registry = crate::commands::Registry::from_catalog(snapshot.prompt_catalog());
                crate::ui::run_with_notice_and_commands(harness, models, notice, registry).await?
            }
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
    match (result, cleanup, diagnostic_cleanup) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (Ok(()), Ok(()), Err(_)) => {
            eprintln!("diagnostic cleanup failed; diagnostics may be incomplete");
            Ok(())
        }
        (Err(primary), _, Err(_)) => {
            Err(primary.context("diagnostic cleanup also failed; diagnostics may be incomplete"))
        }
        (Err(primary), _, Ok(())) => Err(primary),
        (Ok(()), Err(error), Err(_)) => {
            Err(error.context("diagnostic cleanup also failed; diagnostics may be incomplete"))
        }
        (Ok(()), Err(error), Ok(())) => Err(error),
    }
}

fn report_mcp_statuses(statuses: &[McpStatus]) {
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
        eprintln!("MCP {}: {state}", status.alias());
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
        Some(Command::Sessions | Command::Memory { .. } | Command::UndoDream) => {
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
        ],
        Some(
            Command::Login { .. }
            | Command::Logout
            | Command::Config
            | Command::Update { .. }
            | Command::Trust { .. },
        ) => &[],
    };
    categories.iter().copied().collect()
}

fn preflight(cli: &Cli, root: &Directory, data: &Path, snapshot: &ConfigSnapshot) -> Result<()> {
    let applicable = snapshot
        .manifest()
        .filtered(&command_claim_categories(cli.command.as_ref()));
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
    bail!(
        "workspace authority is not approved for this command\n{}\nRun `kuru trust approve` with the same `-C` directory, or repeat this command with `--trust-workspace-once` after review.",
        manifest_text(root, &applicable, Some(store.inspect(snapshot.manifest()))),
    )
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
            ApprovalState::Absent => "not approved",
            ApprovalState::Matching => "approved",
            ApprovalState::Stale => "approval does not match the current manifest",
            ApprovalState::Invalid => "approval state is invalid or unsafe",
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
    file.try_lock().map_err(|error| {
        anyhow::anyhow!(
            "project already has an active Kuru writer, or its lock could not be acquired: {error}"
        )
    })?;
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
    #[cfg(unix)]
    let executable = std::env::current_exe()?;
    #[cfg(unix)]
    let destination = executable.parent().context("executable has no parent")?;
    if let Some(source) = source {
        ensure!(
            version.is_none() && release_base.is_none(),
            "--source cannot be combined with release options"
        );
        #[cfg(unix)]
        {
            let mut command = tokio::process::Command::new("bash");
            command
                .arg(source.join("scripts/install.sh"))
                .arg("--source")
                .env("KURU_INSTALL_DIR", destination);
            let status = command.status().await.context("could not launch updater")?;
            ensure!(status.success(), "source update failed");
        }
        #[cfg(windows)]
        {
            let candidate = build_windows_source(source).await?;
            let outcome =
                kuru_delivery::update::replace_running_binary(&candidate, &update_helper_cache()?)
                    .await?;
            println!("Installed source build at {}", outcome.installed.display());
        }
    } else {
        let version = version
            .context("provide --version VERSION for a verified release or --source CHECKOUT")?;
        let configured_base = std::env::var("KURU_RELEASE_BASE").ok();
        let base = release_base
            .or(configured_base.as_deref())
            .context("--release-base or KURU_RELEASE_BASE is required")?;
        #[cfg(unix)]
        let path = kuru_delivery::archive::install(base, version, destination, None)
            .await
            .context("update failed")?;
        #[cfg(windows)]
        let path = kuru_delivery::update::replace_running(base, version, &update_helper_cache()?)
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

    #[tokio::test]
    async fn automatic_permission_claim_is_reviewed_but_trust_does_not_grant_a_call() {
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
            ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
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
