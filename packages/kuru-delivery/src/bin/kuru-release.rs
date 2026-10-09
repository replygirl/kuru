use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kuru_delivery::{
    homebrew, notes,
    release::{self, GitHub, Version},
    signing,
};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    version,
    about = "Native Kuru release tooling; remote writes require commit or publish"
)]
struct Cli {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Version {
        #[arg(default_value = "auto", value_parser = ["auto", "major", "minor", "patch"])]
        bump: String,
    },
    Plan {
        #[arg(long, default_value = "auto", value_parser = ["auto", "major", "minor", "patch"])]
        bump: String,
    },
    Stamp {
        version: Version,
    },
    Commit {
        #[arg(long)]
        version: Version,
        #[arg(long)]
        expected_sha: String,
    },
    Assemble {
        #[arg(long)]
        version: Version,
        #[arg(long, default_value = "dist")]
        directory: PathBuf,
        #[arg(long)]
        notes: PathBuf,
    },
    Publish {
        #[arg(long)]
        version: Version,
        #[arg(long)]
        sha: String,
        #[arg(long, default_value = "dist")]
        directory: PathBuf,
        #[arg(long)]
        notes: PathBuf,
    },
    Notes {
        #[arg(long)]
        version: Version,
        #[arg(long)]
        sha: String,
        #[arg(long)]
        output: PathBuf,
    },
    SigningPrepare {
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        output: PathBuf,
    },
    SigningVerify {
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        publisher: String,
    },
    RetainedPackage {
        #[arg(long)]
        run_id: u64,
        #[arg(long)]
        version: Version,
        #[arg(long)]
        sha: String,
        #[arg(long)]
        target: String,
    },
    VerifyPackage {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        version: Version,
        #[arg(long)]
        target: String,
    },
    HomebrewGenerate {
        #[arg(long)]
        version: Version,
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        output: PathBuf,
    },
    HomebrewPublish {
        #[arg(long)]
        version: Version,
        #[arg(long)]
        sha: String,
    },
}
fn github() -> Result<GitHub> {
    GitHub::new(
        &std::env::var("GH_REPO").context("GH_REPO is required")?,
        &std::env::var("GH_TOKEN").context("GH_TOKEN is required")?,
    )
}
fn emit(values: &[(&str, &str)]) -> Result<()> {
    let text: String = values
        .iter()
        .map(|(key, value)| format!("{key}={value}\n"))
        .collect();
    if let Some(path) = std::env::var_os("GITHUB_OUTPUT") {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(text.as_bytes())?;
    }
    print!("{text}");
    Ok(())
}
async fn execute(root: &Path, action: Action) -> Result<()> {
    match action {
        Action::Version { bump } => println!("{}", release::compute_version(root, &bump).await?),
        Action::Plan { bump } => {
            let plan = release::plan(root, &bump).await?;
            emit(&[
                ("version", &plan.version),
                ("tag", &plan.tag),
                ("base_sha", &plan.base_sha),
            ])?;
        }
        Action::Stamp { version } => println!(
            "Stamped {}",
            release::stamp(root, version)?
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Action::Commit {
            version,
            expected_sha,
        } => emit(&[(
            "sha",
            &release::commit_version(root, &github()?, &expected_sha, version).await?,
        )])?,
        Action::Assemble {
            version,
            directory,
            notes,
        } => println!(
            "Assembled {} release candidate checks",
            release::assemble(&directory, version, &notes)?
        ),
        Action::Publish {
            version,
            sha,
            directory,
            notes,
        } => println!(
            "{}",
            release::publish(&github()?, &directory, version, &sha, &notes).await?
        ),
        Action::Notes {
            version,
            sha,
            output,
        } => {
            notes::generate(
                root,
                &sha,
                version,
                &output,
                &notes::ProviderOptions::default(),
            )
            .await?;
            println!("{}", output.display());
        }
        Action::SigningPrepare {
            binary,
            target,
            output,
        } => {
            println!("{}", signing::prepare(&binary, &target, &output)?.display());
        }
        Action::SigningVerify {
            binary,
            target,
            publisher,
        } => {
            signing::verify(&binary, &target, &publisher).await?;
        }
        Action::RetainedPackage {
            run_id,
            version,
            sha,
            target,
        } => {
            let found =
                release::retained_package(&github()?, run_id, version, &sha, &target).await?;
            emit(&[
                ("found", if found { "true" } else { "false" }),
                (
                    "name",
                    &release::package_artifact_name(version, &sha, &target)?,
                ),
            ])?;
        }
        Action::VerifyPackage {
            directory,
            version,
            target,
        } => {
            release::verify_package(&directory, version, &target)?;
        }
        Action::HomebrewGenerate {
            version,
            directory,
            repository,
            output,
        } => {
            let hashes = release::verified_assets(&directory, version)?;
            let formula = homebrew::generate(version, &repository, &hashes)?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)?
                .write_all(formula.as_bytes())?;
        }
        Action::HomebrewPublish { version, sha } => {
            let source = github()?;
            let hashes = release::published_assets(&source, version, &sha).await?;
            let formula = homebrew::generate(version, &source.repository, &hashes)?;
            let tap = GitHub::new(
                "replygirl/homebrew-kuru",
                &std::env::var("HOMEBREW_TOKEN").context("HOMEBREW_TOKEN is required")?,
            )?;
            println!(
                "Homebrew formula updated: {}",
                homebrew::publish(&tap, version, &formula).await?
            );
        }
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    execute(&cli.root, cli.command).await
}
