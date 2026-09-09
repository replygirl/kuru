use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use kuru_connectors::{Provider, ToolHost, auth, provider};
use kuru_core::{Config, MemoryStore, Mode, ModelInfo, ProjectPreferences, SelectionOverrides};
use kuru_runtime::Harness;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

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
    #[arg(long, global = true, help = "Allow workspace file mutations")]
    pub allow_write: bool,
    #[arg(
        long,
        global = true,
        help = "Allow shell processes with your process authority (not a sandbox)"
    )]
    pub allow_shell: bool,
    #[arg(long, global = true)]
    pub no_dream: bool,
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
    },
    /// Sign in using the supported Codex OpenAI authentication flow.
    Login {
        #[arg(long)]
        device: bool,
    },
    Logout,
    /// Show authentication status without displaying tokens.
    Auth,
    /// Discover provider models and supported reasoning efforts.
    Models,
    /// Print merged effective configuration.
    Config,
    Sessions,
    /// Consolidate private memories and consider reversible membership changes.
    Dream,
    UndoDream,
    /// Invoke a workspace/MCP tool directly.
    Tool {
        name: String,
        #[arg(long, default_value = "{}")]
        args: String,
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
}

pub fn paths(cli: &Cli) -> Result<(PathBuf, PathBuf, Option<PathBuf>)> {
    let cwd = cli
        .directory
        .canonicalize()
        .context("workspace directory does not exist")?;
    ensure!(cwd.is_dir(), "workspace must be a directory");
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is unset; configure the environment")?;
    let user_config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("kuru/config.toml");
    let user_config = user_config.exists().then_some(user_config);
    let data = cli.data_dir.clone().unwrap_or_else(|| {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("kuru")
    });
    Ok((cwd, data, user_config))
}

pub fn effective_config(
    cli: &Cli,
    cwd: &Path,
    user: Option<&Path>,
    preferences: &ProjectPreferences,
) -> Result<Config> {
    let mut config = Config::load_with_preferences(
        user,
        cwd,
        cli.config.as_deref(),
        preferences,
        SelectionOverrides {
            mode: cli.mode,
            provider: cli.provider.as_deref(),
            model: cli.model.as_deref(),
            effort: cli.effort.as_deref(),
        },
    )?;
    if cli.allow_write {
        config.allow_write = true;
    }
    if cli.allow_shell {
        config.allow_shell = true;
    }
    if cli.no_dream {
        config.dream_every = 0;
        config.dream_on_exit = false;
    }
    config.validate()?;
    Ok(config)
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

pub async fn run() -> Result<()> {
    execute(Cli::parse()).await
}

pub async fn execute(cli: Cli) -> Result<()> {
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
    // A new installation can inspect configuration without creating state. An
    // existing store supplies only this project's interactive choices, never a
    // resumed transcript. Refuse tool-root storage before opening its database.
    let existing_memory = if data.join("memory.sqlite3").try_exists()? {
        ensure!(
            !data.canonicalize()?.starts_with(&cwd),
            "memory directory must be outside the tool workspace; set --data-dir to a separate directory"
        );
        Some(MemoryStore::open(&data.join("memory.sqlite3"))?)
    } else {
        None
    };
    let preferences = existing_memory
        .as_ref()
        .map(|memory| Harness::load_preferences(memory, &cwd))
        .transpose()?
        .unwrap_or_default();
    let mut config = effective_config(&cli, &cwd, user.as_deref(), &preferences)?;
    match &cli.command {
        Some(Command::Login { device: true }) => {
            let status = tokio::process::Command::new(&config.codex_command)
                .args(["login", "--device-auth"])
                .status()
                .await
                .context("install Codex to use ChatGPT authentication")?;
            ensure!(status.success(), "Codex device login failed");
            return Ok(());
        }
        Some(Command::Login { device: false }) => {
            return auth(&config.codex_command, "login").await;
        }
        Some(Command::Logout) => return auth(&config.codex_command, "logout").await,
        Some(Command::Auth) => return auth(&config.codex_command, "status").await,
        Some(Command::Config) => {
            let mut visible = config.clone();
            for server in visible.mcp.values_mut() {
                for value in server.env.values_mut() {
                    *value = "[redacted]".into();
                }
            }
            println!("{}", toml::to_string_pretty(&visible)?);
            return Ok(());
        }
        Some(Command::Tool { name, args }) => {
            let host = ToolHost::new(&cwd, &config)?;
            let result = async {
                let arguments = serde_json::from_str(args)?;
                host.specs().await?;
                host.execute(name, arguments).await
            }
            .await;
            let cleanup = host.shutdown().await;
            println!("{}", result?);
            cleanup?;
            return Ok(());
        }
        Some(Command::Tools) => {
            let host = ToolHost::new(&cwd, &config)?;
            let specs = host.specs().await;
            let cleanup = host.shutdown().await;
            println!("{}", serde_json::to_string_pretty(&specs?)?);
            cleanup?;
            return Ok(());
        }
        _ => {}
    }
    let provider = provider(&config, &cwd)?;
    if matches!(cli.command, Some(Command::Models)) {
        println!(
            "{}",
            serde_json::to_string_pretty(&provider.models().await?)?
        );
        return Ok(());
    }
    let models = if matches!(cli.command, Some(Command::Sessions | Command::UndoDream)) {
        vec![]
    } else {
        select_model(&mut config, &provider).await?
    };
    if matches!(cli.command, Some(Command::Sessions))
        && !data.join("memory.sqlite3").try_exists()?
    {
        println!("[]");
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
        None => MemoryStore::open(&data.join("memory.sqlite3"))?,
    };
    if matches!(cli.command, Some(Command::Sessions)) {
        println!(
            "{}",
            serde_json::to_string_pretty(&Harness::list_sessions(&memory, &cwd)?)?
        );
        return Ok(());
    }
    let _lease = project_lease(&data, &cwd)?;
    let mut harness = Harness::new(config, &cwd, memory, provider, cli.resume.as_deref())?;
    match cli.command {
        Some(Command::Run { prompt, json }) => {
            let result = harness.run(&prompt).await;
            let succeeded = result.is_ok();
            if let Ok(result) = &result {
                if json {
                    println!("{}", serde_json::to_string_pretty(result)?);
                } else {
                    println!("{}", result.text);
                }
            }
            let cleanup = harness.shutdown(succeeded).await;
            result?;
            cleanup?;
        }

        Some(Command::Dream) => {
            println!("{}", serde_json::to_string_pretty(&harness.dream().await?)?);
            harness.shutdown(false).await?;
        }
        Some(Command::UndoDream) => {
            harness.undo_dream()?;
            println!("Previous membership restored.");
        }
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
        None => crate::ui::run(harness, models).await?,
        _ => unreachable!("early-return commands handled above"),
    }
    Ok(())
}

/// Advisory OS locks release when the process exits, including crashes. Keep the
/// file itself: unlinking lockfiles would allow competing locks on new inodes.
fn project_lease(data: &Path, cwd: &Path) -> Result<File> {
    let directory = data.join("locks");
    if let Ok(metadata) = std::fs::symlink_metadata(&directory) {
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "project lock directory must be a regular directory"
        );
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    let digest = Sha256::digest(cwd.as_os_str().as_encoded_bytes());
    let name = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = directory.join(format!("{name}.lock"));
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "project lock must be a regular file"
        );
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .context("cannot open the project writer lock")?;
    file.try_lock().map_err(|error| {
        anyhow::anyhow!(
            "project already has an active Kuru writer, or its lock could not be acquired: {error}"
        )
    })?;
    Ok(file)
}

async fn update(
    version: Option<&str>,
    release_base: Option<&str>,
    source: Option<&Path>,
) -> Result<()> {
    let executable = std::env::current_exe()?;
    let destination = executable.parent().context("executable has no parent")?;
    if let Some(source) = source {
        ensure!(
            version.is_none() && release_base.is_none(),
            "--source cannot be combined with release options"
        );
        let mut command = tokio::process::Command::new("bash");
        command
            .arg(source.join("scripts/install.sh"))
            .arg("--source")
            .env("KURU_INSTALL_DIR", destination);
        let status = command.status().await.context("could not launch updater")?;
        ensure!(
            status.success(),
            "update failed; installed executable retained"
        );
    } else {
        let version = version
            .context("provide --version VERSION for a verified release or --source CHECKOUT")?;
        let configured_base = std::env::var("KURU_RELEASE_BASE").ok();
        let base = release_base
            .or(configured_base.as_deref())
            .context("--release-base or KURU_RELEASE_BASE is required")?;
        let path = kuru_delivery::archive::install(base, version, destination, None)
            .await
            .context("update failed; installed executable retained")?;
        println!(
            "Installed Kuru {} at {}",
            kuru_delivery::archive::checked_version(version)?,
            path.display()
        );
    }
    Ok(())
}
