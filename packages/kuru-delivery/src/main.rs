use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use kuru_delivery::{
    advisory, archive, bundle, coverage, docs, published_windows, repo, shell_support,
};
use std::{ffi::OsString, path::PathBuf};

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
    /// Prepare and validate fail-closed native coverage shard evidence.
    Coverage {
        #[command(subcommand)]
        command: CoverageCommand,
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
    /// Package the five generated shell/man files for one native target.
    PackageShellSupport {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        version: String,
        #[arg(long, default_value = "dist")]
        output: PathBuf,
    },
    /// Verify one exact published Windows release through ordinary mise.
    VerifyPublishedWindows {
        #[arg(long, env = "RELEASE_VERSION")]
        version: String,
        #[arg(long, env = "RELEASE_SHA")]
        expected_sha: String,
        #[arg(long)]
        mise: PathBuf,
        #[arg(long, default_value = "packages/kuru-memory/support/dolt-assets.json")]
        manifest: PathBuf,
        #[arg(long, env = "KURU_PUBLISHED_WINDOWS_RECEIPT")]
        evidence: PathBuf,
        #[arg(long, env = "RELEASE_RUN_URL")]
        run_url: String,
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

#[derive(Subcommand)]
enum CoverageCommand {
    /// Reject a different or modified tracked source tree before compiling.
    VerifySource {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        expected_source: String,
        #[arg(long)]
        llvm_cov: PathBuf,
    },
    /// Canonicalize the full instrumented Cargo artifact inventory.
    Inventory {
        #[arg(long)]
        metadata: PathBuf,
        #[arg(long)]
        messages: PathBuf,
        #[arg(long)]
        target_dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify that a package selection reuses the full artifact inventory.
    Selection {
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long, value_delimiter = ',')]
        packages: Vec<String>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Write an exact Cargo native-runner configuration for one shard.
    RunnerConfig {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        host: String,
        #[arg(long)]
        helper: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        target_dir: PathBuf,
        #[arg(long)]
        ledger: PathBuf,
        /// Existing directory for per-test output logs and stall reports.
        #[arg(long)]
        diagnostics: PathBuf,
        /// Hosted job start in Unix seconds.
        #[arg(long)]
        job_started: u64,
        /// Hosted job `timeout-minutes` limit.
        #[arg(long)]
        job_minutes: u64,
        #[arg(long)]
        output: PathBuf,
    },
    /// Dispatch one Cargo-selected test and record its exact shard action.
    Dispatch {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        target_dir: PathBuf,
        #[arg(long)]
        ledger: PathBuf,
        #[arg(long)]
        diagnostics: PathBuf,
        /// Shard test deadline in Unix seconds.
        #[arg(long)]
        deadline: u64,
        executable: PathBuf,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Require Cargo to have invoked every full-inventory test exactly once.
    ValidateRun {
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        ledger: PathBuf,
    },
    /// Remove only compile-phase profiles before accepting uploaded test profiles.
    DiscardCompileProfiles {
        #[arg(long)]
        profiles: PathBuf,
    },
    /// Bind successful raw profiles to their exact source and artifact inventory.
    Receipt {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        ledger: PathBuf,
        #[arg(long)]
        profiles: PathBuf,
        #[arg(long)]
        shard: String,
        #[arg(long)]
        run_attempt: String,
        #[arg(long)]
        expected_source: String,
        #[arg(long)]
        llvm_cov: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify each shard's latest attempt and copy only accepted profiles for reporting.
    Collect {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        inputs: PathBuf,
        #[arg(long)]
        target_dir: PathBuf,
        #[arg(long)]
        expected_source: String,
        /// The current workflow run attempt; no shard may claim a later one.
        #[arg(long)]
        max_attempt: String,
        #[arg(long)]
        llvm_cov: PathBuf,
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
        Command::Coverage {
            command:
                CoverageCommand::VerifySource {
                    root,
                    expected_source,
                    llvm_cov,
                },
        } => {
            coverage::verify_source(&root, &expected_source, &llvm_cov).await?;
        }
        Command::Coverage {
            command:
                CoverageCommand::Inventory {
                    metadata,
                    messages,
                    target_dir,
                    output,
                },
        } => {
            coverage::write_inventory(&metadata, &messages, &target_dir, &output)?;
        }
        Command::Coverage {
            command:
                CoverageCommand::Selection {
                    inventory,
                    packages,
                    output,
                },
        } => {
            coverage::write_selection(&inventory, &packages, &output)?;
        }
        Command::Coverage {
            command:
                CoverageCommand::RunnerConfig {
                    root,
                    host,
                    helper,
                    inventory,
                    selection,
                    target_dir,
                    ledger,
                    diagnostics,
                    job_started,
                    job_minutes,
                    output,
                },
        } => {
            coverage::write_runner_config(&coverage::RunnerConfigOptions {
                root: &root,
                host: &host,
                helper: &helper,
                inventory: &inventory,
                selection: &selection,
                target_dir: &target_dir,
                ledger: &ledger,
                diagnostics: &diagnostics,
                job_started,
                job_minutes,
                output: &output,
            })?;
        }
        Command::Coverage {
            command:
                CoverageCommand::Dispatch {
                    root,
                    inventory,
                    selection,
                    target_dir,
                    ledger,
                    diagnostics,
                    deadline,
                    executable,
                    args,
                },
        } => {
            let status = coverage::dispatch_test(&coverage::DispatchOptions {
                root: &root,
                inventory: &inventory,
                selection: &selection,
                target_dir: &target_dir,
                ledger: &ledger,
                diagnostics: &diagnostics,
                deadline,
                executable: &executable,
                args: &args,
            })
            .await?;
            if let Some(status) = status
                && !status.success()
            {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Command::Coverage {
            command:
                CoverageCommand::ValidateRun {
                    inventory,
                    selection,
                    ledger,
                },
        } => {
            coverage::validate_run_ledger(&inventory, &selection, &ledger)?;
        }
        Command::Coverage {
            command: CoverageCommand::DiscardCompileProfiles { profiles },
        } => {
            println!("{}", coverage::discard_compile_profiles(&profiles)?);
        }
        Command::Coverage {
            command:
                CoverageCommand::Receipt {
                    root,
                    inventory,
                    selection,
                    ledger,
                    profiles,
                    shard,
                    run_attempt,
                    expected_source,
                    llvm_cov,
                    output,
                },
        } => {
            coverage::write_receipt(&coverage::ReceiptOptions {
                root: &root,
                inventory: &inventory,
                selection: &selection,
                ledger: &ledger,
                profiles: &profiles,
                shard: &shard,
                run_attempt: &run_attempt,
                expected_source: &expected_source,
                llvm_cov: &llvm_cov,
                output: &output,
            })
            .await?;
        }
        Command::Coverage {
            command:
                CoverageCommand::Collect {
                    root,
                    inventory,
                    inputs,
                    target_dir,
                    expected_source,
                    max_attempt,
                    llvm_cov,
                },
        } => {
            let selected = coverage::collect_profiles(
                &root,
                &inventory,
                &inputs,
                &target_dir,
                &expected_source,
                &max_attempt,
                &llvm_cov,
            )
            .await?;
            for (shard, attempt) in selected {
                println!("coverage shard {shard}: accepted run attempt {attempt}");
            }
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
        Command::PackageShellSupport {
            input,
            target,
            version,
            output,
        } => {
            println!(
                "{}",
                shell_support::package(&input, &target, &version, &output)?.display()
            );
        }
        Command::VerifyPublishedWindows {
            version,
            expected_sha,
            mise,
            manifest,
            evidence,
            run_url,
        } => {
            published_windows::run(published_windows::Options {
                version,
                expected_sha,
                mise,
                manifest,
                evidence,
                run_url,
            })
            .await?;
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
