use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use kuru_delivery::{advisory, archive, bundle, docs, repo};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Kuru installation and repository delivery checks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Refresh or scan the isolated public RustSec advisory database.
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
    /// Prepare verified local build inputs without compiling their consumer.
    Bundle {
        #[command(subcommand)]
        command: BundleCommand,
    },
    /// Install a checksum-verified release atomically.
    Install {
        #[arg(long)]
        version: String,
        #[arg(long, env = "KURU_RELEASE_BASE")]
        release_base: String,
        #[arg(long, env = "KURU_INSTALL_DIR")]
        install_dir: Option<PathBuf>,
        #[arg(long)]
        target: Option<String>,
    },
    /// Install a trusted local source build without executing it.
    InstallLocal {
        #[arg(long)]
        binary: PathBuf,
        #[arg(long, env = "KURU_INSTALL_DIR")]
        install_dir: PathBuf,
        #[arg(long)]
        target: Option<String>,
    },
    /// Create a reproducible release archive and its checksum.
    Package {
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        version: String,
        #[arg(long, default_value = "dist")]
        output: PathBuf,
    },
    /// Check public artifacts, base paths, links, anchors and sitemap.
    Docs {
        #[arg(long, default_value = "apps/kuru-docs/.vitepress/dist")]
        root: PathBuf,
        #[arg(long, env = "KURU_DOCS_BASE", default_value = "/kuru/")]
        base: String,
    },
    /// Check repository architecture and metadata invariants.
    Repo {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

#[derive(Subcommand)]
enum AuditCommand {
    /// Explicitly create or refresh the public advisory database checkout.
    Refresh {
        #[arg(long, env = "KURU_ADVISORY_DB")]
        database: PathBuf,
    },
    /// Scan the root lockfile without fetching advisory data.
    Scan {
        #[arg(long, env = "KURU_ADVISORY_DB")]
        database: PathBuf,
        #[arg(long)]
        audit_bin: PathBuf,
    },
}

#[derive(Subcommand)]
enum BundleCommand {
    Prepare {
        #[arg(long, default_value = "packages/kuru-memory/support/dolt-assets.json")]
        manifest: PathBuf,
        #[arg(long, env = "KURU_DOLT_BUNDLE_TARGET", default_value = "host")]
        target: String,
        #[arg(long, env = "KURU_DOLT_BUNDLE_DIR")]
        bundle_dir: Option<PathBuf>,
        #[arg(long, env = "KURU_DOLT_BUNDLE_ARCHIVE")]
        archive: Option<PathBuf>,
        #[arg(long, env = "KURU_DOLT_BUNDLE_OFFLINE")]
        offline: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Audit {
            command: AuditCommand::Refresh { database },
        } => {
            advisory::refresh(&database).await?;
        }
        Command::Audit {
            command:
                AuditCommand::Scan {
                    database,
                    audit_bin,
                },
        } => {
            advisory::scan(&database, &audit_bin).await?;
        }
        Command::Bundle {
            command:
                BundleCommand::Prepare {
                    manifest,
                    target,
                    bundle_dir,
                    archive,
                    offline,
                },
        } => {
            let bundle_dir = bundle::bundle_directory(&manifest, bundle_dir)?;
            let prepared = bundle::prepare(&bundle::PrepareOptions {
                manifest,
                target,
                bundle_dir,
                archive,
                offline,
            })
            .await?;
            println!("{}", prepared.display());
        }
        Command::Install {
            version,
            release_base,
            install_dir,
            target,
        } => {
            let directory = install_dir
                .or_else(default_install_dir)
                .context("provide --install-dir when the user installation directory is unset")?;
            let installed =
                archive::install(&release_base, &version, &directory, target.as_deref()).await?;
            println!(
                "Installed Kuru {} at {}",
                archive::checked_version(&version)?,
                installed.display()
            );
        }
        Command::Package {
            binary,
            target,
            version,
            output,
        } => {
            println!(
                "{}",
                archive::package(&binary, &target, &version, &output)?.display()
            );
        }
        Command::InstallLocal {
            binary,
            install_dir,
            target,
        } => {
            println!(
                "{}",
                archive::install_local(&binary, &install_dir, target.as_deref())?.display()
            );
        }
        Command::Docs { root, base } => {
            let errors = docs::check(&root, &base)?;
            ensure!(errors.is_empty(), "{}", errors.join("\n"));
            println!("Public docs artifacts, local links and anchors passed ({base}).");
        }
        Command::Repo { root } => {
            let errors = repo::check(&root)?;
            ensure!(errors.is_empty(), "{}", errors.join("\n"));
            println!("Repository metadata invariants passed.");
        }
    }
    Ok(())
}

fn default_install_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("Programs/kuru/bin"));
    #[cfg(unix)]
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/bin"))
}
