use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use kuru_delivery::{archive, docs, repo};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Kuru installation and repository delivery checks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Install {
            version,
            release_base,
            install_dir,
            target,
        } => {
            let directory = install_dir
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/bin"))
                })
                .context("provide --install-dir when HOME is unset")?;
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
