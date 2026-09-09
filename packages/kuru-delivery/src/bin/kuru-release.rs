use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kuru_delivery::{
    notes,
    release::{self, GitHub, Version},
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
        #[arg(long, default_value = "")]
        resume_version: String,
        #[arg(long, default_value = "")]
        resume_sha: String,
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
        Action::Plan {
            bump,
            resume_version,
            resume_sha,
        } => {
            let plan = release::plan(root, &bump, &resume_version, &resume_sha).await?;
            emit(&[
                ("version", &plan.version),
                ("tag", &plan.tag),
                ("base_sha", &plan.base_sha),
                ("resume", if plan.resume { "true" } else { "false" }),
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
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    execute(&cli.root, cli.command).await
}
