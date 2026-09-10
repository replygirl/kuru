use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use kuru_connectors::{AuthAction, Provider, ToolHost, auth, provider};
use kuru_core::{Config, Mode, ModelInfo, ProjectPreferences, SelectionOverrides};
use kuru_memory::{MemoryStore, OpenOptions as MemoryOptions};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
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

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// Show the active project, engine version and revision.
    Status,
    /// List recent committed memory revisions.
    History {
        #[arg(long, default_value_t = 20)]
        limit: usize,
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
    let scope = kuru_runtime::project_scope(&cwd)?;
    let memory_config = Config::load_memory(user.as_deref(), &cwd, cli.config.as_deref())?;
    let writer = matches!(
        cli.command,
        None | Some(
            Command::Run { .. } | Command::Dream | Command::UndoDream | Command::Serve { .. }
        )
    );
    let exists = MemoryStore::exists(&data, &scope)?;
    let legacy_path = data.join("memory.sqlite3");
    let legacy = match std::fs::symlink_metadata(&legacy_path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("cannot inspect legacy memory"),
    };
    let migrate = legacy && !exists;
    let _lease = if writer || migrate {
        Directory::ensure_private(&data)?;
        ensure_outside_workspace(&data, &cwd)?;
        Some(project_lease(&data, &cwd)?)
    } else {
        None
    };
    // A new installation can inspect configuration without creating state. An
    // existing store supplies only this project's interactive choices, never a
    // resumed transcript. Refuse tool-root storage before opening its database.
    let existing_memory = if exists || legacy {
        ensure_outside_workspace(&data, &cwd)?;
        let mut options = MemoryOptions::new(data.clone(), scope.clone());
        options.config = memory_config.clone();
        options.read_only = !writer && !migrate;
        Some(MemoryStore::open(options).await?)
    } else {
        None
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
        let mut config = effective_config(&cli, &cwd, user.as_deref(), &preferences)?;
        match &cli.command {
            Some(Command::Login { device: true }) => {
                return auth(&config.codex_command, AuthAction::DeviceLogin).await;
            }
            Some(Command::Login { device: false }) => {
                return auth(&config.codex_command, AuthAction::Login).await;
            }
            Some(Command::Logout) => return auth(&config.codex_command, AuthAction::Logout).await,
            Some(Command::Auth) => return auth(&config.codex_command, AuthAction::Status).await,
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
                }
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
        let models = if matches!(cli.command, Some(Command::UndoDream)) {
            vec![]
        } else {
            select_model(&mut config, &provider).await?
        };
        std::fs::create_dir_all(&data)?;
        let data = data.canonicalize()?;
        ensure!(
            !data.starts_with(&cwd),
            "memory directory must be outside the tool workspace; set --data-dir to a separate directory"
        );
        let memory = match existing_memory {
            Some(memory) => memory,
            None => {
                let mut options = MemoryOptions::new(data, scope);
                options.config = memory_config;
                MemoryStore::open(options).await?
            }
        };
        memory_to_close = Some(memory.clone());
        let mut harness = Harness::new(config, &cwd, memory, provider, cli.resume.as_deref()).await?;
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
                let result = harness.dream().await;
                let cleanup = harness.shutdown(false).await;
                println!("{}", serde_json::to_string_pretty(&result?)?);
                cleanup?;
            }
            Some(Command::UndoDream) => {
                let result = harness.undo_dream().await;
                let cleanup = harness.shutdown(false).await;
                result?;
                cleanup?;
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
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let cleanup = if let Some(memory) = memory_to_close {
        memory.close().await
    } else {
        Ok(())
    };
    result?;
    cleanup
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
