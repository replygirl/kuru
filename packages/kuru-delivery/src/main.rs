use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use kuru_delivery::{
    advisory, archive, bundle, coverage, docs, open_time, published_windows, repo, shell_support,
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
    /// Run and merge fail-closed partitioned native test and coverage evidence.
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
        /// Windows target to verify; must be this runner's native host target.
        #[arg(long, env = "KURU_PUBLISHED_TARGET")]
        target: Option<String>,
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
    /// Measure a release kuru executable's memory open time from outside,
    /// report only: no time is compared with a budget.
    OpenTime {
        #[arg(long, env = "KURU_OPEN_TIME_BINARY")]
        binary: PathBuf,
        #[arg(long, env = "KURU_OPEN_TIME_OUTPUT")]
        output: PathBuf,
        /// Parent of the private scratch root (default: the temporary directory).
        #[arg(long, env = "KURU_OPEN_TIME_SCRATCH")]
        scratch: Option<PathBuf>,
        /// Iterations; each runs the three cases once.
        #[arg(long, env = "KURU_OPEN_TIME_ITERATIONS", default_value_t = 10)]
        iterations: usize,
        /// Host label for the summary, such as the CI runner label.
        #[arg(long, env = "KURU_OPEN_TIME_LABEL")]
        label: Option<String>,
        /// Also append the summary to this file (a CI job summary).
        #[arg(long, env = "GITHUB_STEP_SUMMARY")]
        summary: Option<PathBuf>,
        /// Sampling period of the process and file observer, in milliseconds.
        #[arg(long, env = "KURU_OPEN_TIME_INTERVAL_MS", default_value_t = 20)]
        interval_ms: u64,
        /// `off` runs the control series: processes only, no file listing.
        #[arg(long, env = "KURU_OPEN_TIME_FILES", default_value = "on", value_parser = ["on", "off"])]
        files: String,
        /// `off` runs the ramp series: no wait for owner retirement between
        /// runs, one wait after the last.
        #[arg(long, env = "KURU_OPEN_TIME_RETIRE_WAIT", default_value = "on", value_parser = ["on", "off"])]
        retire_wait: String,
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
    /// Build a pinned source-built engine archive on the linux-x64 build host.
    Build {
        #[arg(long, default_value = "packages/kuru-memory/support/dolt-assets.json")]
        manifest: PathBuf,
        #[arg(long)]
        target: String,
        /// Fresh private work directory (must not exist).
        #[arg(long)]
        work_dir: PathBuf,
        /// Private directory receiving `<stem>.zip` and `pins.json`.
        #[arg(long)]
        output: PathBuf,
        /// Report observed pins; required while the manifest is unpinned.
        #[arg(long)]
        print_pins: bool,
        #[arg(long, env = "KURU_DOLT_BUNDLE_OFFLINE")]
        offline: bool,
        /// Non-authoritative local iteration on another host.
        #[arg(
            long,
            env = "KURU_BUNDLE_BUILD_HOST_OVERRIDE",
            value_parser = clap::builder::BoolishValueParser::new()
        )]
        host_override: bool,
    },
}

#[derive(Subcommand)]
#[cfg_attr(
    windows,
    expect(
        clippy::large_enum_variant,
        reason = "Windows' wider PathBuf pushes this CLI enum past the size threshold; it is parsed once per process, so its size has no cost"
    )
)]
enum CoverageCommand {
    /// Dispatch one Cargo-selected test executable: list its tests and run
    /// this partition's share with exact selections (the Cargo runner).
    Dispatch {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        /// Rust host target, which keys host-specific exclusions.
        #[arg(long)]
        host: String,
        /// One-based partition index.
        #[arg(long)]
        partition: u32,
        /// Partition count of this OS.
        #[arg(long)]
        partitions: u32,
        #[arg(long)]
        target_dir: PathBuf,
        #[arg(long)]
        ledger: PathBuf,
        #[arg(long)]
        diagnostics: PathBuf,
        /// Partition test deadline in Unix seconds.
        #[arg(long)]
        deadline: u64,
        executable: PathBuf,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Run one fail-closed test partition from its `KURU_COVERAGE_*` inputs.
    Shard {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Run the scoped packages without coverage instrumentation.
        #[arg(long)]
        uninstrumented: bool,
    },
    /// Require one OS's partition receipts to agree and be complete, then
    /// enforce its coverage gate, from the `KURU_COVERAGE_*` inputs.
    Merge {
        #[arg(long, default_value = ".")]
        root: PathBuf,
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
        Command::Bundle {
            command:
                BundleCommand::Build {
                    manifest,
                    target,
                    work_dir,
                    output,
                    print_pins,
                    offline,
                    host_override,
                },
        } => {
            let report = bundle::build::build(&bundle::build::BuildOptions {
                manifest,
                target,
                work_dir,
                output,
                print_pins,
                offline,
                host_override,
                host: bundle::build::Host::current(),
                jobs: std::thread::available_parallelism().map_or(2, usize::from),
            })
            .await?;
            if print_pins {
                println!("{}", serde_json::to_string_pretty(&report.pins)?);
            }
            println!("{}", bundle::build::summary(&report));
        }
        Command::Coverage {
            command:
                CoverageCommand::Dispatch {
                    root,
                    inventory,
                    host,
                    partition,
                    partitions,
                    target_dir,
                    ledger,
                    diagnostics,
                    deadline,
                    executable,
                    args,
                },
        } => {
            let partition = coverage::partition::PartitionScheme::new(partition, partitions)?;
            let status = coverage::dispatch_test(&coverage::DispatchOptions {
                root: &root,
                inventory: &inventory,
                host: &host,
                partition: &partition,
                target_dir: &target_dir,
                ledger: &ledger,
                diagnostics: &diagnostics,
                deadline,
                executable: &executable,
                args: &args,
            })
            .await;
            // A runner stopped by a signal has already terminated its test
            // group; report the signal as a failed runner to Cargo.
            #[cfg(unix)]
            if let Err(error) = &status
                && let Some(interrupted) = error.downcast_ref::<coverage::RunnerInterrupted>()
            {
                eprintln!("Error: {error}");
                std::process::exit(interrupted.exit_code());
            }
            let status = status?;
            if let Some(status) = status
                && !status.success()
            {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Command::Coverage {
            command:
                CoverageCommand::Shard {
                    root,
                    uninstrumented,
                },
        } => {
            let mode = if uninstrumented {
                coverage::Mode::Uninstrumented
            } else {
                coverage::Mode::Instrumented
            };
            coverage::orchestrate::shard(&root, mode).await?;
        }
        Command::Coverage {
            command: CoverageCommand::Merge { root },
        } => {
            coverage::orchestrate::merge(&root).await?;
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
            target,
        } => {
            published_windows::run(published_windows::Options {
                version,
                expected_sha,
                mise,
                manifest,
                evidence,
                run_url,
                target,
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
        Command::OpenTime {
            binary,
            output,
            scratch,
            iterations,
            label,
            summary,
            interval_ms,
            files,
            retire_wait,
        } => {
            let mut options = open_time::Options::new(binary, output);
            options.scratch = scratch;
            options.iterations = iterations;
            if let Some(label) = label {
                options.label = label;
            }
            options.summary = summary.filter(|path| !path.as_os_str().is_empty());
            options.interval = std::time::Duration::from_millis(interval_ms);
            options.files = files == "on";
            options.retire_wait = retire_wait == "on";
            let report = open_time::run(&options).await?;
            println!("{}", report.summary);
            println!(
                "Recorded {} runs ({} failed to open, {} failed after opening); no budget is applied.",
                report.records, report.failed_opens, report.failed_after_open
            );
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
