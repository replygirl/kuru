//! One cross-platform orchestrator for a test partition and its per-OS merge.
//!
//! `coverage shard` compiles the partition's inventory into a fresh target,
//! optionally seeded with allow-listed dependency artifacts, runs it through
//! the task-private Cargo runner that lists every executable and runs only
//! this partition's tests, exports the partition's LCOV when instrumented and
//! writes its receipt. `coverage merge` accepts an OS's partitions only when
//! their receipts agree and their plans are disjoint and complete, then
//! enforces that OS's 90% line gate on the union of their LCOV.
//!
//! Every process this module starts goes through [`Host`], so the sequencing
//! and each refusal are unit-tested with a fake. The coverage environment from
//! `cargo-llvm-cov show-env` is applied only to those children: the helper's
//! own process, which is also Cargo's runner, is never instrumented.

use super::{
    COMMAND_OUTPUT_LIMIT, LLVM_COV_VERSION, Mode, RUNNER_LEDGER_FILE, ReceiptOptions,
    RunnerConfigOptions, WORKSPACE_PACKAGES, artifact_os_label, canonical_attempt,
    check_partitioning, discard_compile_profiles, lcov, ledger, merge, partition::PartitionScheme,
    plan, seed, unix_now, workspace_identity, write_inventory, write_json, write_new,
    write_runner_config,
};
use crate::command;
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

/// The pinned cargo-llvm-cov tool request resolved through mise.
const LLVM_COV_TOOL: &str = "aqua:taiki-e/cargo-llvm-cov@0.9.1";
/// Bound for short captured commands. `show-env` resolves full Cargo metadata,
/// which may fetch dependency manifests on a cold runner.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Private state directory created inside the fresh coverage target.
const STATE: &str = "kuru-shard-state";
/// State copied beside `failure.txt` when a partition fails after creating it.
const DIAGNOSTIC_COPIES: [&str; 4] = [
    "inventory.json",
    "runner-config.toml",
    RUNNER_LEDGER_FILE,
    "job-ledger.json",
];
const INPUT_PREFIX: &str = "KURU_COVERAGE_";
/// Process variables that change what Cargo builds or how tests run. Their
/// values, with the coverage environment, form the profile-environment digest.
const PROFILE_ENV: [&str; 8] = [
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_INCREMENTAL",
    "CARGO_PROFILE_DEV_DEBUG",
    "CARGO_PROFILE_TEST_DEBUG",
    "KURU_TEST_SUPERVISOR_PREPARED",
    "RUSTFLAGS",
    "RUST_TEST_THREADS",
];
/// The token that replaces the target path inside the digested environment,
/// so partitions with different target paths can still agree.
const TARGET_TOKEN: &str = "${KURU_COVERAGE_TARGET}";

/// Run one partition from the process's `KURU_COVERAGE_*` inputs.
pub async fn shard(root: &Path, mode: Mode) -> Result<()> {
    // `cargo run` built this helper in the ordinary target before any coverage
    // environment existed; it is the Cargo runner and is never rebuilt here.
    let helper = std::env::current_exe().context("locate the coverage helper")?;
    run(
        Step::Shard(mode),
        root,
        &helper,
        std::env::vars_os(),
        &mut System,
    )
    .await
}

/// Merge one OS's partitions from the process's `KURU_COVERAGE_*` inputs.
pub async fn merge(root: &Path) -> Result<()> {
    let helper = std::env::current_exe().context("locate the coverage helper")?;
    run(Step::Merge, root, &helper, std::env::vars_os(), &mut System).await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    Shard(Mode),
    Merge,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Self::Shard(_) => "shard",
            Self::Merge => "merge",
        }
    }

    fn required(self) -> &'static [&'static str] {
        match self {
            Self::Shard(Mode::Instrumented) => &[
                "TARGET",
                "SOURCE",
                "ATTEMPT",
                "OS",
                "PARTITION",
                "PARTITIONS",
                "OUTPUT",
                "DIAGNOSTICS",
                "JOB_STARTED",
                "JOB_MINUTES",
            ],
            Self::Shard(Mode::Uninstrumented) => &[
                "TARGET",
                "SOURCE",
                "ATTEMPT",
                "OS",
                "PARTITION",
                "PARTITIONS",
                "PACKAGES",
                "OUTPUT",
                "DIAGNOSTICS",
                "JOB_STARTED",
                "JOB_MINUTES",
            ],
            Self::Merge => &[
                "SOURCE",
                "ATTEMPT",
                "OS",
                "MODE",
                "PARTITIONS",
                "INPUTS",
                "REPORT",
            ],
        }
    }
}

#[derive(Debug)]
struct Common {
    source: String,
    attempt: String,
    os: String,
}

#[derive(Debug)]
struct ShardInputs {
    mode: Mode,
    target: PathBuf,
    partition: u64,
    partitions: u64,
    /// Sorted package scope; the whole workspace when instrumented.
    packages: Vec<String>,
    output: PathBuf,
    diagnostics: PathBuf,
    job_started: u64,
    job_minutes: u64,
    seed: Option<PathBuf>,
    seed_export: Option<PathBuf>,
    helper_cache: Option<String>,
    seed_cache: Option<String>,
    seed_key: Option<String>,
}

#[derive(Debug)]
struct MergeInputs {
    mode: Mode,
    /// The package scope every partition must have built.
    packages: Vec<String>,
    partitions: u64,
    inputs: PathBuf,
    report: PathBuf,
}

#[derive(Debug)]
enum Plan {
    Shard(ShardInputs),
    Merge(MergeInputs),
}

#[derive(Debug)]
struct Inputs {
    common: Common,
    plan: Plan,
    /// The process variables named by [`PROFILE_ENV`], as found.
    profile_env: BTreeMap<String, Option<String>>,
}

impl Inputs {
    /// Require every input of the step to be present and well formed. Values
    /// are read from the supplied environment, never the process's own.
    fn parse(step: Step, vars: impl IntoIterator<Item = (OsString, OsString)>) -> Result<Self> {
        let mut values = BTreeMap::new();
        let mut profile_env: BTreeMap<String, Option<String>> = PROFILE_ENV
            .iter()
            .map(|name| ((*name).to_owned(), None))
            .collect();
        for (key, value) in vars {
            if let Some(name) = key.to_str()
                && let Some(slot) = profile_env.get_mut(name)
            {
                *slot = Some(value.to_string_lossy().into_owned());
            }
            let Some(name) = key.to_str().and_then(|key| key.strip_prefix(INPUT_PREFIX)) else {
                continue;
            };
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("{INPUT_PREFIX}{name} is not UTF-8"))?;
            if !value.trim().is_empty() {
                values.insert(name.to_owned(), value);
            }
        }
        let missing: Vec<_> = step
            .required()
            .iter()
            .filter(|name| !values.contains_key(**name))
            .map(|name| format!("{INPUT_PREFIX}{name}"))
            .collect();
        ensure!(
            missing.is_empty(),
            "coverage {} requires {}",
            step.name(),
            missing.join(", ")
        );
        let text = |name: &str| values[name].clone();
        let path = |name: &str| {
            std::path::absolute(&values[name])
                .with_context(|| format!("resolve {INPUT_PREFIX}{name}"))
        };
        let optional_path = |name: &str| values.contains_key(name).then(|| path(name)).transpose();
        let number = |name: &str| {
            values[name]
                .trim()
                .parse::<u64>()
                .with_context(|| format!("{INPUT_PREFIX}{name} must be a non-negative integer"))
        };
        let common = Common {
            source: text("SOURCE"),
            attempt: text("ATTEMPT"),
            os: text("OS"),
        };
        let packages = |mode: Mode| -> Result<Vec<String>> {
            match mode {
                Mode::Instrumented => {
                    ensure!(
                        !values.contains_key("PACKAGES"),
                        "an instrumented partition builds the whole workspace; unset {INPUT_PREFIX}PACKAGES"
                    );
                    Ok(WORKSPACE_PACKAGES.map(str::to_owned).to_vec())
                }
                Mode::Uninstrumented => {
                    let listed = values.get("PACKAGES").with_context(|| {
                        format!("uninstrumented mode requires {INPUT_PREFIX}PACKAGES")
                    })?;
                    let mut packages: Vec<_> = listed
                        .split(',')
                        .map(str::trim)
                        .filter(|package| !package.is_empty())
                        .map(str::to_owned)
                        .collect();
                    packages.sort();
                    packages.dedup();
                    Ok(packages)
                }
            }
        };
        let plan = match step {
            Step::Shard(mode) => {
                let packages = packages(mode)?;
                Plan::Shard(ShardInputs {
                    mode,
                    target: path("TARGET")?,
                    partition: number("PARTITION")?,
                    partitions: number("PARTITIONS")?,
                    packages,
                    output: path("OUTPUT")?,
                    diagnostics: path("DIAGNOSTICS")?,
                    job_started: number("JOB_STARTED")?,
                    job_minutes: number("JOB_MINUTES")?,
                    seed: optional_path("SEED")?,
                    seed_export: optional_path("SEED_EXPORT")?,
                    helper_cache: values.get("HELPER_CACHE").cloned(),
                    seed_cache: values.get("SEED_CACHE").cloned(),
                    seed_key: values.get("SEED_MATCHED_KEY").cloned(),
                })
            }
            Step::Merge => {
                let mode = Mode::parse(values["MODE"].trim())?;
                Plan::Merge(MergeInputs {
                    mode,
                    packages: packages(mode)?,
                    partitions: number("PARTITIONS")?,
                    inputs: path("INPUTS")?,
                    report: path("REPORT")?,
                })
            }
        };
        Ok(Self {
            common,
            plan,
            profile_env,
        })
    }
}

/// One child process request. `env` is applied over the child's inherited
/// environment only; the orchestrator never changes its own environment.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Invocation {
    program: OsString,
    args: Vec<OsString>,
    cwd: PathBuf,
    env: Vec<(OsString, OsString)>,
}

impl Invocation {
    fn new(program: impl AsRef<OsStr>, args: &[&str], cwd: &Path) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: args.iter().map(OsString::from).collect(),
            cwd: cwd.to_owned(),
            env: Vec::new(),
        }
    }

    fn with_env(mut self, env: &[(OsString, OsString)]) -> Self {
        self.env = env.to_vec();
        self
    }

    fn describe(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.args)
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Every process interaction of the orchestrator.
trait Host {
    /// Run a short command and return its trimmed standard output. Output is
    /// bounded and the command is timed out.
    async fn capture(&mut self, invocation: &Invocation) -> Result<String>;
    /// Run a command to completion without a timeout. Standard output goes to
    /// a new file or, without one, to this job's log; standard error is
    /// inherited. Compilation and the test run are not under the partition
    /// deadline: the Cargo runner enforces that per test selection.
    async fn stream(&mut self, invocation: &Invocation, stdout: Option<&Path>) -> Result<()>;
    /// Reject a different or modified tracked source (git, rustc, cargo).
    async fn verify_source(
        &mut self,
        root: &Path,
        source: &str,
        llvm_cov: Option<&Path>,
    ) -> Result<()>;
    /// Bind the partition's evidence to its exact identity (git, rustc, cargo).
    async fn receipt(&mut self, options: &ReceiptOptions<'_>) -> Result<()>;
}

/// The real process boundary.
struct System;

impl Host for System {
    async fn capture(&mut self, invocation: &Invocation) -> Result<String> {
        let mut child = command::rooted(&invocation.cwd, &invocation.program);
        child.args(&invocation.args);
        child.envs(invocation.env.iter().map(|(key, value)| (key, value)));
        let output = command::bounded_output(&mut child, CAPTURE_TIMEOUT, COMMAND_OUTPUT_LIMIT)
            .await
            .with_context(|| format!("run {}", invocation.describe()))?;
        ensure!(
            output.status.success(),
            "{} failed with {}: {}",
            invocation.describe(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(String::from_utf8(output.stdout)
            .with_context(|| format!("{} printed non-UTF-8 output", invocation.describe()))?
            .trim()
            .to_owned())
    }

    async fn stream(&mut self, invocation: &Invocation, stdout: Option<&Path>) -> Result<()> {
        let status = stream_child(invocation, stdout)
            .await
            .with_context(|| format!("run {}", invocation.describe()))?;
        ensure!(
            status.success(),
            "{} failed with {status}",
            invocation.describe()
        );
        Ok(())
    }

    async fn verify_source(
        &mut self,
        root: &Path,
        source: &str,
        llvm_cov: Option<&Path>,
    ) -> Result<()> {
        super::verify_source(root, source, llvm_cov).await
    }

    async fn receipt(&mut self, options: &ReceiptOptions<'_>) -> Result<()> {
        super::write_receipt(options).await
    }
}

fn new_output_file(path: &Path) -> Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))
}

#[cfg(unix)]
async fn stream_child(
    invocation: &Invocation,
    stdout: Option<&Path>,
) -> Result<std::process::ExitStatus> {
    use std::process::Stdio;

    // Cargo drives its own compiler and test children to completion; the
    // established Tokio command API waits for it without a deadline.
    let mut child = tokio::process::Command::new(&invocation.program);
    child
        .args(&invocation.args)
        .current_dir(&invocation.cwd)
        .envs(invocation.env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit());
    match stdout {
        Some(path) => child.stdout(new_output_file(path)?),
        None => child.stdout(Stdio::inherit()),
    };
    Ok(child.status().await?)
}

#[cfg(windows)]
async fn stream_child(
    invocation: &Invocation,
    stdout: Option<&Path>,
) -> Result<std::process::ExitStatus> {
    use kuru_platform::windows::process::{
        Lifetime, StandardStream, Stdio, configured_command, inherited_stdio, merge_environment,
    };
    use std::os::windows::io::OwnedHandle;

    let environment = merge_environment(std::env::vars_os(), invocation.env.clone())?;
    let mut spec = configured_command(
        &invocation.program,
        &invocation.args,
        &invocation.cwd,
        environment,
    )?;
    // Like the shell launch this replaces, Cargo is awaited as a plain child:
    // no additional Job layer is placed between it and the runner's own
    // per-test Jobs, whose memory fixture may request native breakaway.
    spec.lifetime = Lifetime::TrustedSupervisor;
    spec.stdin = inherited_stdio(StandardStream::Input)?;
    spec.stdout = match stdout {
        Some(path) => Stdio::Handle(OwnedHandle::from(new_output_file(path)?)),
        None => inherited_stdio(StandardStream::Output)?,
    };
    spec.stderr = inherited_stdio(StandardStream::Error)?;
    let mut child = spec.spawn().await?;
    loop {
        match child.wait(Duration::from_secs(60 * 60)).await {
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            result => return Ok(result?),
        }
    }
}

async fn run<H: Host>(
    step: Step,
    root: &Path,
    helper: &Path,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    host: &mut H,
) -> Result<()> {
    let Inputs {
        common,
        plan,
        profile_env,
    } = Inputs::parse(step, vars)?;
    match plan {
        Plan::Shard(inputs) => {
            // Every later failure, including toolchain and inventory setup,
            // leaves its error and the partition state beside the per-test logs.
            create_fresh_directory(&inputs.diagnostics, "coverage diagnostics")?;
            let diagnostics = resolved_directory(&inputs.diagnostics)?;
            let mut state = None;
            let result = run_shard(
                host,
                root,
                helper,
                &common,
                &inputs,
                &profile_env,
                &diagnostics,
                &mut state,
            )
            .await;
            if let Err(error) = &result {
                record_failure(&diagnostics, state.as_deref(), error);
            }
            result
        }
        Plan::Merge(inputs) => run_merge(&common, &inputs),
    }
}

/// Write `failure.txt` and copy the partition's manifests and ledgers. A
/// failure to record diagnostics is reported but never replaces its error.
fn record_failure(diagnostics: &Path, state: Option<&Path>, error: &anyhow::Error) {
    if let Err(write) = fs::write(diagnostics.join("failure.txt"), format!("{error:?}\n")) {
        eprintln!("coverage diagnostics: write failure.txt failed: {write}");
    }
    let Some(state) = state else {
        return;
    };
    for name in DIAGNOSTIC_COPIES {
        let source = state.join(name);
        if fs::symlink_metadata(&source).is_ok_and(|metadata| metadata.file_type().is_file())
            && let Err(copy) = fs::copy(&source, diagnostics.join(name))
        {
            eprintln!("coverage diagnostics: copy {name} failed: {copy}");
        }
    }
}

/// Create exactly this directory, refusing one that already exists.
fn create_fresh_directory(path: &Path, label: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::create_dir(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!("{label} already exists: {}", path.display())
        } else {
            anyhow::Error::new(error).context(format!("create {label} {}", path.display()))
        }
    })
}

/// An absolute directory path as Cargo and the runner will see it: physical
/// on Unix, and on Windows absolute without a verbatim prefix.
fn resolved_directory(path: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    let resolved = fs::canonicalize(path).with_context(|| format!("resolve {}", path.display()))?;
    #[cfg(windows)]
    let resolved =
        std::path::absolute(path).with_context(|| format!("resolve {}", path.display()))?;
    ensure!(
        resolved.is_absolute() && fs::symlink_metadata(&resolved)?.file_type().is_dir(),
        "{} is not an absolute directory",
        resolved.display()
    );
    Ok(resolved)
}

/// Phase timestamps of the job ledger.
#[derive(Default)]
struct Phases(BTreeMap<String, ledger::Phase>);

impl Phases {
    fn record(&mut self, name: &str, started: u64) -> Result<()> {
        let finished = unix_now()?.max(started);
        self.0
            .insert(name.to_owned(), ledger::Phase { started, finished });
        Ok(())
    }
}

/// The prepared inventory of one partition.
struct Prepared {
    root: PathBuf,
    target: PathBuf,
    state: PathBuf,
    llvm_cov: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
    inventory: PathBuf,
    messages: PathBuf,
    workspace: super::WorkspaceIdentity,
    profile_env_sha256: String,
    seed: ledger::CacheLedger,
}

/// Digest of the build and test environment with the target path replaced by
/// a token: equal for partitions of one OS whatever their target paths.
fn profile_env_sha256(
    profile_env: &BTreeMap<String, Option<String>>,
    coverage: &[(String, String)],
    targets: &[&str],
) -> Result<String> {
    let neutral = |value: &str| {
        targets
            .iter()
            .filter(|target| !target.is_empty())
            .fold(value.to_owned(), |value, target| {
                value.replace(target, TARGET_TOKEN)
            })
    };
    let mut digested = BTreeMap::new();
    for (name, value) in profile_env {
        digested.insert(
            format!("env:{name}"),
            value.as_deref().map_or("<unset>".to_owned(), neutral),
        );
    }
    for (name, value) in coverage {
        digested.insert(format!("show-env:{name}"), neutral(value));
    }
    super::digest_json(&digested)
}

/// Package arguments for Cargo: the workspace, or each scoped package.
fn package_args(mode: Mode, packages: &[String]) -> Vec<String> {
    match mode {
        Mode::Instrumented => vec!["--workspace".to_owned()],
        Mode::Uninstrumented => packages
            .iter()
            .flat_map(|package| ["-p".to_owned(), package.clone()])
            .collect(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn prepare<H: Host>(
    host: &mut H,
    root: &Path,
    inputs: &ShardInputs,
    common: &Common,
    profile_env: &BTreeMap<String, Option<String>>,
    phases: &mut Phases,
    state_slot: &mut Option<PathBuf>,
) -> Result<Prepared> {
    let started = unix_now()?;
    let root = resolved_directory(root).context("coverage root")?;
    // One coverage writer per target: a partition never reuses an existing tree.
    create_fresh_directory(&inputs.target, "coverage target")?;
    let target = resolved_directory(&inputs.target)?;
    let state = target.join(STATE);
    fs::create_dir(&state).with_context(|| format!("create {}", state.display()))?;
    *state_slot = Some(state.clone());
    let target_text = target
        .to_str()
        .context("coverage target path is not UTF-8")?
        .to_owned();

    let mut env = vec![(OsString::from("CARGO_TARGET_DIR"), target.clone().into())];
    let mut coverage = Vec::new();
    let mut shown_target = String::new();
    let llvm_cov = match inputs.mode {
        Mode::Instrumented => {
            let bin = host
                .capture(&Invocation::new(
                    "mise",
                    &["bin-paths", LLVM_COV_TOOL],
                    &root,
                ))
                .await?;
            let mut lines = bin.lines().filter(|line| !line.trim().is_empty());
            let (Some(bin), None) = (lines.next(), lines.next()) else {
                bail!("mise did not name one {LLVM_COV_TOOL} directory: {bin:?}");
            };
            let llvm_cov = Path::new(bin.trim())
                .join(format!("cargo-llvm-cov{}", std::env::consts::EXE_SUFFIX));
            ensure!(
                llvm_cov.is_absolute()
                    && fs::metadata(&llvm_cov).is_ok_and(|metadata| metadata.is_file()),
                "pinned cargo-llvm-cov executable is missing: {}",
                llvm_cov.display()
            );
            let version = host
                .capture(&Invocation::new(
                    &llvm_cov,
                    &["llvm-cov", "--version"],
                    &root,
                ))
                .await?;
            ensure!(
                version == LLVM_COV_VERSION,
                "expected {LLVM_COV_VERSION}, got {version}"
            );
            host.verify_source(&root, &common.source, Some(&llvm_cov))
                .await?;
            env.push((
                OsString::from("CARGO_LLVM_COV_TARGET_DIR"),
                target.clone().into(),
            ));
            env.push((
                OsString::from("LLVM_PROFILE_FILE"),
                target.join("kuru-%p-%m.profraw").into(),
            ));
            let shown = host
                .capture(
                    &Invocation::new(&llvm_cov, &["llvm-cov", "show-env", "--pwsh"], &root)
                        .with_env(&env),
                )
                .await?;
            coverage =
                parse_show_env(&shown).context("apply the cargo-llvm-cov coverage environment")?;
            shown_target = coverage
                .iter()
                .find_map(|(name, value)| (name == "CARGO_LLVM_COV_TARGET_DIR").then_some(value))
                .context("cargo-llvm-cov show-env omits CARGO_LLVM_COV_TARGET_DIR")?
                .clone();
            ensure!(
                Path::new(&shown_target) == target,
                "cargo-llvm-cov selected target {shown_target}, expected {}",
                target.display()
            );
            // The coverage environment is applied last, as a shell evaluating
            // `show-env` would: its LLVM_PROFILE_FILE names this target's profiles.
            for (name, value) in &coverage {
                env.retain(|(key, _)| key != name.as_str());
                env.push((name.into(), value.into()));
            }
            Some(llvm_cov)
        }
        Mode::Uninstrumented => {
            host.verify_source(&root, &common.source, None).await?;
            None
        }
    };
    let profile_env_sha256 =
        profile_env_sha256(profile_env, &coverage, &[&shown_target, &target_text])?;

    let metadata = state.join("metadata.json");
    host.stream(
        &Invocation::new(
            "cargo",
            &["metadata", "--format-version=1", "--no-deps", "--locked"],
            &root,
        )
        .with_env(&env),
        Some(&metadata),
    )
    .await?;
    let workspace = workspace_identity(&metadata)?;
    ensure!(
        workspace.packages == WORKSPACE_PACKAGES,
        "workspace packages {:?} differ from WORKSPACE_PACKAGES {WORKSPACE_PACKAGES:?}",
        workspace.packages
    );
    for package in &inputs.packages {
        ensure!(
            workspace.packages.contains(package),
            "unknown coverage package {package}"
        );
    }
    phases.record("prepare", started)?;

    // Seeded dependency units enter the fresh target before any Cargo child
    // writes into it. A missing seed is a miss and an unreadable entry a
    // counted refusal, never an error.
    let started = unix_now()?;
    let names = seed::workspace_names(workspace.names.iter().map(String::as_str));
    let report = match &inputs.seed {
        Some(path) => seed::import(path, &target, &names)
            .with_context(|| format!("import the dependency seed {}", path.display()))?,
        None => None,
    };
    let seed_state = ledger::seed_state(
        inputs.seed.is_some(),
        report.is_some(),
        inputs.seed_cache.as_deref(),
    );
    let report = report.unwrap_or_default();
    eprintln!(
        "coverage partition: dependency seed {seed_state}; imported {} entries ({} bytes); refused {:?}",
        report.transferred.entries, report.transferred.bytes, report.refused
    );
    let seed = ledger::CacheLedger {
        helper: ledger::cache_state(inputs.helper_cache.as_deref()).to_owned(),
        seed: seed_state.to_owned(),
        seed_matched_key: inputs.seed_key.clone(),
        seed_imported: report.transferred,
        seed_refused: report.refused,
        dependency_units_rebuilt: 0,
        workspace_units_rebuilt: 0,
    };
    phases.record("seed_import", started)?;

    let started = unix_now()?;
    let messages = state.join("full-messages.json");
    let mut args = vec!["test".to_owned()];
    args.extend(package_args(inputs.mode, &inputs.packages));
    args.extend(
        [
            "--all-targets",
            "--all-features",
            "--locked",
            "--no-run",
            "--message-format=json-render-diagnostics",
        ]
        .map(str::to_owned),
    );
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    host.stream(
        &Invocation::new("cargo", &args, &root).with_env(&env),
        Some(&messages),
    )
    .await?;
    let inventory = state.join("inventory.json");
    write_inventory(&metadata, &messages, &target, &inputs.packages, &inventory)
        .context("coverage inventory validation failed")?;
    phases.record("compile", started)?;
    Ok(Prepared {
        root,
        target,
        state,
        llvm_cov,
        env,
        inventory,
        messages,
        workspace,
        profile_env_sha256,
        seed,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_shard<H: Host>(
    host: &mut H,
    root: &Path,
    helper: &Path,
    common: &Common,
    inputs: &ShardInputs,
    profile_env: &BTreeMap<String, Option<String>>,
    diagnostics: &Path,
    state_slot: &mut Option<PathBuf>,
) -> Result<()> {
    ensure!(
        helper.is_absolute()
            && fs::symlink_metadata(helper).is_ok_and(|metadata| metadata.file_type().is_file()),
        "coverage helper {} is not an absolute regular file",
        helper.display()
    );
    ensure!(
        canonical_attempt(&common.attempt).is_some(),
        "coverage run attempt must be a positive integer without leading zeros"
    );
    let count = u32::try_from(inputs.partitions).context("partition count is too large")?;
    let index = u32::try_from(inputs.partition).context("partition index is too large")?;
    let partition = PartitionScheme::new(index, count)?;
    check_partitioning(&common.os, inputs.mode, count)?;
    let known = super::workspace_packages();
    ensure!(
        !inputs.packages.is_empty(),
        "coverage partition has no packages"
    );
    for package in &inputs.packages {
        ensure!(
            known.contains(package.as_str()),
            "unknown coverage package {package}"
        );
    }
    if let Some(export) = &inputs.seed_export {
        ensure!(
            partition.index == 1,
            "only partition 1 exports the dependency seed"
        );
        ensure!(
            fs::symlink_metadata(export).is_err(),
            "seed export already exists: {}",
            export.display()
        );
    }

    let mut phases = Phases::default();
    let prepared = prepare(
        host,
        root,
        inputs,
        common,
        profile_env,
        &mut phases,
        state_slot,
    )
    .await?;
    let Prepared {
        root,
        target,
        state,
        llvm_cov,
        env,
        inventory,
        messages,
        workspace,
        profile_env_sha256,
        seed: mut cache,
    } = prepared;

    let rustc = host
        .capture(&Invocation::new("rustc", &["-vV"], &root))
        .await?;
    let hosts: Vec<_> = rustc
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect();
    let [host_target] = hosts[..] else {
        bail!("Rust identity did not contain one host target");
    };
    if inputs.mode == Mode::Uninstrumented {
        // Real-memory tests resolve the prepared supervisor snapshot beside
        // their own executables, so it is prepared inside this target, as the
        // memory package's test task prepares it before its tests.
        let started = unix_now()?;
        host.stream(
            &Invocation::new(
                "cargo",
                &[
                    "run",
                    "-p",
                    "kuru-memory",
                    "--features",
                    "test-support",
                    "--locked",
                    "--",
                    "prefetch",
                ],
                &root,
            )
            .with_env(&env),
            None,
        )
        .await
        .context("prepare the memory supervisor snapshot")?;
        phases.record("prefetch", started)?;
    }
    let ledger_path = state.join(RUNNER_LEDGER_FILE);
    let runner_config = state.join("runner-config.toml");
    // The runner's test deadline sits inside the hosted job limit, so a
    // stalled test fails here with evidence instead of a host cancellation.
    write_runner_config(&RunnerConfigOptions {
        root: &root,
        host: host_target,
        helper,
        inventory: &inventory,
        partition: &partition,
        target_dir: &target,
        ledger: &ledger_path,
        diagnostics,
        job_started: inputs.job_started,
        job_minutes: inputs.job_minutes,
        output: &runner_config,
    })
    .context("Cargo runner configuration failed")?;
    // Compile-phase profiles are not test evidence.
    let discarded =
        discard_compile_profiles(&target).context("compile-profile isolation failed")?;
    eprintln!(
        "coverage partition {} of {}: discarded {discarded} compile-phase profiles",
        partition.index, partition.count
    );
    let config = runner_config
        .to_str()
        .context("Cargo runner configuration path is not UTF-8")?;
    let started = unix_now()?;
    let mut args = vec!["--config".to_owned(), config.to_owned(), "test".to_owned()];
    args.extend(package_args(inputs.mode, &inputs.packages));
    args.extend(
        [
            "--all-targets",
            "--all-features",
            "--locked",
            "--no-fail-fast",
        ]
        .map(str::to_owned),
    );
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    host.stream(&Invocation::new("cargo", &args, &root).with_env(&env), None)
        .await
        .context("coverage partition tests failed")?;
    phases.record("tests", started)?;
    let plan = super::validate_run_ledger(&inventory, &partition, host_target, &ledger_path)
        .context("Cargo runner ledger validation failed")?;
    let tests: usize = plan
        .executables
        .iter()
        .map(|executable| executable.assigned.len())
        .sum();
    eprintln!(
        "coverage partition {} of {}: ran {tests} tests from {} executables",
        partition.index,
        partition.count,
        plan.executables.len()
    );

    let (profile_count, profile_bytes) = profile_totals(&target)?;
    let lcov_path = match &llvm_cov {
        Some(llvm_cov) => {
            let started = unix_now()?;
            ensure!(
                profile_count <= super::PROFILE_COUNT_LIMIT
                    && profile_bytes <= super::PROFILE_TOTAL_LIMIT,
                "coverage partition profiles exceed their limits ({profile_count} files, {profile_bytes} bytes)"
            );
            let raw = state.join("coverage.raw.lcov");
            let raw_text = raw.to_str().context("LCOV path is not UTF-8")?;
            host.stream(
                &Invocation::new(
                    llvm_cov,
                    &[
                        "llvm-cov",
                        "report",
                        "--failure-mode",
                        "any",
                        "--lcov",
                        "--output-path",
                        raw_text,
                    ],
                    &root,
                )
                .with_env(&env),
                None,
            )
            .await
            .context("partition coverage export failed")?;
            let bytes = super::read_bounded(&raw, lcov::LCOV_LIMIT)
                .context("partition coverage export was not written")?;
            let normalized = lcov::Lcov::parse(
                std::str::from_utf8(&bytes).context("partition LCOV is not UTF-8")?,
            )?
            .normalize(root.to_str().context("coverage root is not UTF-8")?)?;
            let path = state.join("coverage.lcov");
            write_new(&path, normalized.render().as_bytes())?;
            let totals = normalized.totals();
            eprintln!(
                "coverage partition {} of {}: {}% of lines in this partition alone ({} of {}); the gate applies to the merge",
                partition.index,
                partition.count,
                lcov::percent(&totals),
                totals.lines_hit,
                totals.lines_found
            );
            phases.record("lcov_export", started)?;
            Some(path)
        }
        None => None,
    };

    let (dependencies, workspace_units) = ledger::rebuilt_units(&messages, &workspace.ids)?;
    cache.dependency_units_rebuilt = dependencies;
    cache.workspace_units_rebuilt = workspace_units;
    let records = plan::read_ledger(&ledger_path)?;
    let job_ledger = state.join("job-ledger.json");
    write_json(
        &job_ledger,
        &ledger::JobLedger {
            schema: super::SCHEMA,
            partition: partition.clone(),
            mode: inputs.mode,
            job_started: inputs.job_started,
            phases: phases.0,
            cache,
            profiles: ledger::profiles(profile_count, profile_bytes),
            executables: ledger::executables(&records),
        },
    )?;
    // The receipt writes attempt-<n> inside the artifact root, so the uploaded
    // artifact names its own attempt in either download layout.
    host.receipt(&ReceiptOptions {
        root: &root,
        mode: inputs.mode,
        partition: &partition,
        inventory: &inventory,
        ledger: &ledger_path,
        job_ledger: &job_ledger,
        profiles: &target,
        lcov: lcov_path.as_deref(),
        run_attempt: &common.attempt,
        expected_source: &common.source,
        llvm_cov: llvm_cov.as_deref(),
        profile_env_sha256: &profile_env_sha256,
        output: &inputs.output,
    })
    .await
    .context("coverage partition receipt failed")?;

    // Export after the receipt, so it can never affect evidence. A failed
    // export only leaves an empty seed and a slower next run.
    if let Some(export) = &inputs.seed_export {
        let started = unix_now()?;
        let names = seed::workspace_names(workspace.names.iter().map(String::as_str));
        match seed::export(&target, export, &names) {
            Ok(report) => eprintln!(
                "coverage partition: exported {} seed entries ({} bytes) in {} s; refused {:?}",
                report.transferred.entries,
                report.transferred.bytes,
                unix_now()?.saturating_sub(started),
                report.refused
            ),
            Err(error) => {
                eprintln!("coverage partition: seed export failed and is left empty: {error:#}");
                let _ = fs::remove_dir_all(export);
                fs::create_dir_all(export)
                    .with_context(|| format!("create {}", export.display()))?;
            }
        }
    }
    Ok(())
}

/// Count and size of the raw profiles in the target root.
fn profile_totals(target: &Path) -> Result<(usize, u64)> {
    let mut count = 0;
    let mut bytes = 0_u64;
    for entry in fs::read_dir(target)? {
        let entry = entry?;
        if entry.path().extension() == Some(OsStr::new("profraw")) {
            count += 1;
            bytes = bytes.saturating_add(fs::symlink_metadata(entry.path())?.len());
        }
    }
    Ok((count, bytes))
}

fn run_merge(common: &Common, inputs: &MergeInputs) -> Result<()> {
    artifact_os_label(&common.os)?;
    let count = u32::try_from(inputs.partitions).context("partition count is too large")?;
    let summary = merge::merge(&merge::MergeOptions {
        inputs: &inputs.inputs,
        report: &inputs.report,
        os: &common.os,
        source: &common.source,
        attempt: &common.attempt,
        count,
        mode: inputs.mode,
        scope: &inputs.packages,
    })
    .with_context(|| format!("coverage merge for {} failed", common.os))?;
    merge::print_summary(&summary);
    Ok(())
}

/// Parse `cargo-llvm-cov llvm-cov show-env --pwsh` output exactly.
///
/// cargo-llvm-cov 0.9.1 writes one line per variable,
/// `$env:NAME="<value>"`, where every character of the value is written as
/// its Rust Unicode escape with `\` replaced by a backtick (`` `u{2f} ``).
/// That form is identical on every OS, unlike the default output, whose shell
/// quoting follows the host (`shell_escape::escape`). Anything else fails.
fn parse_show_env(text: &str) -> Result<Vec<(String, String)>> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    ensure!(!body.is_empty(), "cargo-llvm-cov show-env printed nothing");
    let mut seen = BTreeSet::new();
    let mut variables = Vec::new();
    for (index, line) in body.split('\n').enumerate() {
        let number = index + 1;
        let (name, value) = show_env_line(line)
            .with_context(|| format!("unexpected show-env line {number}: {line:?}"))?;
        ensure!(
            seen.insert(name.to_owned()),
            "show-env line {number} repeats {name}"
        );
        variables.push((name.to_owned(), value));
    }
    Ok(variables)
}

fn show_env_line(line: &str) -> Result<(&str, String)> {
    let assignment = line
        .strip_prefix("$env:")
        .context("line is not a PowerShell environment assignment")?;
    let (name, quoted) = assignment
        .split_once('=')
        .context("assignment has no value")?;
    let mut bytes = name.bytes();
    ensure!(
        bytes
            .next()
            .is_some_and(|first| first.is_ascii_uppercase() || first == b'_')
            && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
        "variable name {name:?} is not [A-Z_][A-Z0-9_]*"
    );
    let escaped = quoted
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .context("value is not one double-quoted string")?;
    let mut value = String::new();
    let mut rest = escaped;
    while !rest.is_empty() {
        let escape = rest
            .strip_prefix("`u{")
            .context("value holds a character outside a `u{..} escape")?;
        let (hex, tail) = escape.split_once('}').context("unterminated escape")?;
        let code = u32::from_str_radix(hex, 16).ok().filter(|code| {
            // Exactly the lowercase, unpadded form Rust's escape_unicode writes.
            format!("{code:x}") == hex
        });
        let character = code
            .and_then(char::from_u32)
            .filter(|character| *character != '\0')
            .with_context(|| format!("invalid escape `u{{{hex}}}"))?;
        value.push(character);
        rest = tail;
    }
    Ok((name, value))
}

#[cfg(test)]
mod tests {
    use super::super::{DispatchOptions, Inventory, dispatch_with, fixture, read_json};
    use super::*;
    use tempfile::TempDir;

    const MACOS_CAPTURED: &str =
        include_str!("../../tests/fixtures/coverage-show-env/macos-captured.pwsh.txt");
    const LINUX_CAPTURED: &str =
        include_str!("../../tests/fixtures/coverage-show-env/linux-captured.pwsh.txt");
    const WINDOWS_CAPTURED: &str =
        include_str!("../../tests/fixtures/coverage-show-env/windows-captured.pwsh.txt");

    fn pwsh(name: &str, value: &str) -> String {
        let escaped: String = value
            .chars()
            .map(|character| format!("`u{{{:x}}}", u32::from(character)))
            .collect();
        format!("$env:{name}=\"{escaped}\"")
    }

    fn variables(text: &str) -> BTreeMap<String, String> {
        parse_show_env(text).unwrap().into_iter().collect()
    }

    #[test]
    fn show_env_fixtures_parse_on_every_os() {
        // Captured on macOS arm64 with the pinned 0.9.1 and a throwaway target.
        let macos = variables(MACOS_CAPTURED);
        let target = "/private/tmp/kuru-coverage-show-env/target";
        assert_eq!(macos["CARGO_LLVM_COV_TARGET_DIR"], target);
        assert_eq!(macos["CARGO_LLVM_COV_BUILD_DIR"], target);
        assert_eq!(
            macos["LLVM_PROFILE_FILE"],
            format!("{target}/ci-coverage-shard-orchestrator-%p-%14m.profraw")
        );
        assert_eq!(
            macos["__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS"],
            "-C\u{1f}instrument-coverage\u{1f}--cfg=coverage"
        );
        assert!(macos["RUSTC_WRAPPER"].ends_with("/0.9.1/bin/cargo-llvm-cov"));
        assert!(
            macos["__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES"]
                .split(',')
                .any(|name| name == "kuru_delivery")
        );
        let names_of = |text: &str| -> Vec<String> {
            parse_show_env(text)
                .unwrap()
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        };
        let names = names_of(MACOS_CAPTURED);
        assert_eq!(
            names,
            [
                "LLVM_PROFILE_FILE",
                "__CARGO_LLVM_COV_RUSTC_WRAPPER",
                "__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS",
                "__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES",
                "RUSTC_WRAPPER",
                "CARGO_LLVM_COV",
                "CARGO_LLVM_COV_SHOW_ENV",
                "CARGO_LLVM_COV_TARGET_DIR",
                "CARGO_LLVM_COV_BUILD_DIR",
            ]
        );

        // Captured on GitHub-hosted ubuntu-latest and windows-latest runners
        // (CI run 36276764569, cargo-llvm-cov 0.9.1) from the stdout of
        // `show-env --pwsh` with the target in `runner.temp`. The Windows
        // target keeps the mixed separators of the runner-built path.
        let linux = variables(LINUX_CAPTURED);
        let target = "/home/runner/work/_temp/kuru-coverage-show-env-1";
        assert_eq!(linux["CARGO_LLVM_COV_TARGET_DIR"], target);
        assert_eq!(linux["CARGO_LLVM_COV_BUILD_DIR"], target);
        assert_eq!(
            linux["LLVM_PROFILE_FILE"],
            format!("{target}/kuru-%p-%4m.profraw")
        );
        assert_eq!(
            linux["RUSTC_WRAPPER"],
            "/home/runner/.local/share/mise/installs/cargo-cargo-llvm-cov/0.9.1/bin/cargo-llvm-cov"
        );
        assert_eq!(
            linux["__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS"],
            macos["__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS"]
        );
        assert!(
            linux["__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES"]
                .split(',')
                .any(|name| name == "kuru_delivery")
        );
        assert_eq!(names_of(LINUX_CAPTURED), names);
        let windows = variables(WINDOWS_CAPTURED);
        let target = r"D:\a\_temp/kuru-coverage-show-env-1";
        assert_eq!(windows["CARGO_LLVM_COV_TARGET_DIR"], target);
        assert_eq!(windows["CARGO_LLVM_COV_BUILD_DIR"], target);
        assert_eq!(
            windows["LLVM_PROFILE_FILE"],
            format!(r"{target}\kuru-%p-%4m.profraw")
        );
        assert_eq!(
            windows["RUSTC_WRAPPER"],
            r"C:\Users\runneradmin\AppData\Local\mise\installs\cargo-cargo-llvm-cov\0.9.1\bin\cargo-llvm-cov.exe"
        );
        assert_eq!(
            windows["__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS"],
            macos["__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS"]
        );
        assert_eq!(
            windows["__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES"],
            linux["__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES"]
        );
        assert_eq!(names_of(WINDOWS_CAPTURED), names);
        // The trimmed captures parse identically.
        for capture in [MACOS_CAPTURED, LINUX_CAPTURED, WINDOWS_CAPTURED] {
            assert_eq!(variables(capture.trim_end()), variables(capture));
        }
    }

    #[test]
    fn show_env_rejects_anything_but_exact_escapes() {
        let good = pwsh("CARGO_LLVM_COV", "1");
        assert_eq!(
            parse_show_env(&good).unwrap(),
            [("CARGO_LLVM_COV".to_owned(), "1".to_owned())]
        );
        assert_eq!(
            parse_show_env("$env:EMPTY=\"\"").unwrap(),
            [("EMPTY".to_owned(), String::new())]
        );
        for (text, reason) in [
            ("", "printed nothing"),
            ("\n", "printed nothing"),
            ("CARGO_LLVM_COV=1", "unexpected show-env line 1"),
            ("export CARGO_LLVM_COV=1", "unexpected show-env line 1"),
            ("$env:CARGO_LLVM_COV='1'", "unexpected show-env line 1"),
            ("$env:CARGO_LLVM_COV=\"1\"", "unexpected show-env line 1"),
            (
                "$env:CARGO_LLVM_COV=\"\\u{31}\"",
                "unexpected show-env line 1",
            ),
            ("$env:CARGO_LLVM_COV=\"`n\"", "unexpected show-env line 1"),
            ("$env:CARGO_LLVM_COV=\"`u{31}", "unexpected show-env line 1"),
            (
                "$env:CARGO_LLVM_COV=\"`u{31\"",
                "unexpected show-env line 1",
            ),
            (
                "$env:CARGO_LLVM_COV=\"`u{031}\"",
                "unexpected show-env line 1",
            ),
            (
                "$env:CARGO_LLVM_COV=\"`u{4A}\"",
                "unexpected show-env line 1",
            ),
            ("$env:CARGO_LLVM_COV=\"`u{}\"", "unexpected show-env line 1"),
            (
                "$env:CARGO_LLVM_COV=\"`u{d800}\"",
                "unexpected show-env line 1",
            ),
            (
                "$env:CARGO_LLVM_COV=\"`u{110000}\"",
                "unexpected show-env line 1",
            ),
            (
                "$env:CARGO_LLVM_COV=\"`u{0}\"",
                "unexpected show-env line 1",
            ),
            (
                "$env:cargo_llvm_cov=\"`u{31}\"",
                "unexpected show-env line 1",
            ),
            ("$env:1CARGO=\"`u{31}\"", "unexpected show-env line 1"),
            ("$env:=\"`u{31}\"", "unexpected show-env line 1"),
            ("$env:CARGO_LLVM_COV", "unexpected show-env line 1"),
            (
                "$env:CARGO_LLVM_COV=\"`u{31}\"\r",
                "unexpected show-env line 1",
            ),
        ] {
            let error = format!("{:#}", parse_show_env(text).unwrap_err());
            assert!(error.contains(reason), "{text:?}: {error}");
        }
        let error = format!(
            "{:#}",
            parse_show_env(&format!("{good}\n\n{good}")).unwrap_err()
        );
        assert!(error.contains("unexpected show-env line 2"), "{error}");
        let error = format!(
            "{:#}",
            parse_show_env(&format!("{good}\n{good}")).unwrap_err()
        );
        assert!(error.contains("line 2 repeats CARGO_LLVM_COV"), "{error}");
    }

    const HOST_TRIPLE: &str = fixture::HOST;
    const SEED_HASH: &str = "0123456789abcdef";

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Call {
        BinPaths,
        Version,
        VerifySource,
        ShowEnv,
        Metadata,
        NoRun,
        Rustc,
        Prefetch,
        TestRun,
        Export,
        Receipt,
    }

    /// Records every requested process interaction and simulates its effect:
    /// Cargo's build and runner, the coverage export and the receipt.
    struct Fake {
        workspace: PathBuf,
        llvm_cov_dir: PathBuf,
        calls: Vec<Call>,
        invocations: Vec<Invocation>,
        fail: Option<Call>,
        version: String,
        show_env: Option<String>,
        bin_paths: Option<String>,
        rustc: String,
        packages: Vec<&'static str>,
        verified: Vec<Option<PathBuf>>,
        receipts: Vec<(Mode, PartitionScheme, String, PathBuf)>,
        write_export: bool,
        /// Whether the seeded dependency was already in the target at build.
        seeded_at_build: Option<bool>,
    }

    impl Fake {
        fn new(temp: &Path) -> Self {
            let workspace = temp.join("workspace");
            for package in WORKSPACE_PACKAGES {
                let directory = workspace.join("packages").join(package);
                fs::create_dir_all(directory.join("src")).unwrap();
                fs::write(
                    directory.join("Cargo.toml"),
                    format!("[package]\nname = \"{package}\"\nversion = \"0.0.0\"\n"),
                )
                .unwrap();
            }
            let llvm_cov_dir = temp.join("llvm-cov-bin");
            fs::create_dir_all(&llvm_cov_dir).unwrap();
            fs::write(
                llvm_cov_dir.join(format!("cargo-llvm-cov{}", std::env::consts::EXE_SUFFIX)),
                b"",
            )
            .unwrap();
            Self {
                workspace: resolved_directory(&workspace).unwrap(),
                llvm_cov_dir,
                calls: Vec::new(),
                invocations: Vec::new(),
                fail: None,
                version: LLVM_COV_VERSION.to_owned(),
                show_env: None,
                bin_paths: None,
                rustc: format!("rustc 1.98.1\nhost: {HOST_TRIPLE}"),
                packages: WORKSPACE_PACKAGES.to_vec(),
                verified: Vec::new(),
                receipts: Vec::new(),
                write_export: true,
                seeded_at_build: None,
            }
        }

        fn call(&mut self, call: Call) -> Result<()> {
            self.calls.push(call);
            ensure!(self.fail != Some(call), "injected {call:?} failure");
            Ok(())
        }

        fn env<'a>(invocation: &'a Invocation, name: &str) -> Option<&'a OsStr> {
            invocation
                .env
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_os_str())
        }

        fn target(invocation: &Invocation) -> PathBuf {
            PathBuf::from(Self::env(invocation, "CARGO_TARGET_DIR").expect("coverage target env"))
        }

        fn crate_name(package: &str) -> String {
            package.replace('-', "_")
        }

        fn metadata(&self) -> serde_json::Value {
            let packages: Vec<_> = self
                .packages
                .iter()
                .map(|package| {
                    serde_json::json!({
                        "id": format!("{package}-id"),
                        "name": package,
                        "manifest_path": self.workspace.join("packages").join(package).join("Cargo.toml"),
                        "targets": [{"kind": ["lib"], "name": Self::crate_name(package), "test": true}],
                    })
                })
                .collect();
            serde_json::json!({
                "packages": packages,
                "workspace_members": self.packages.iter().map(|package| format!("{package}-id")).collect::<Vec<_>>(),
                "workspace_root": self.workspace,
            })
        }

        /// Cargo's messages for the requested packages, writing their files.
        fn messages(&self, invocation: &Invocation) -> Vec<u8> {
            let target = Self::target(invocation);
            let args: Vec<_> = invocation
                .args
                .iter()
                .map(|arg| arg.to_str().unwrap())
                .collect();
            let scope: Vec<&str> = if args.contains(&"--workspace") {
                self.packages.clone()
            } else {
                args.windows(2)
                    .filter(|pair| pair[0] == "-p")
                    .map(|pair| pair[1])
                    .collect()
            };
            let deps = target.join("debug/deps");
            fs::create_dir_all(&deps).unwrap();
            // A dependency and a workspace library, as a real build leaves them.
            fs::write(deps.join(format!("libserde-{SEED_HASH}.rlib")), b"serde").unwrap();
            fs::write(deps.join(format!("libkuru_core-{SEED_HASH}.rlib")), b"core").unwrap();
            let mut bytes = Vec::new();
            let mut message = |value: serde_json::Value| {
                serde_json::to_writer(&mut bytes, &value).unwrap();
                bytes.push(b'\n');
            };
            message(serde_json::json!({
                "reason": "compiler-artifact", "package_id": "serde-id", "fresh": false,
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "serde",
                    "src_path": "/registry/serde/src/lib.rs", "edition": "2021",
                    "doc": true, "doctest": true, "test": false},
                "profile": {"opt_level": "0", "debuginfo": 0, "debug_assertions": true,
                    "overflow_checks": true, "test": false},
                "features": [], "filenames": [deps.join(format!("libserde-{SEED_HASH}.rlib"))],
                "executable": null
            }));
            for package in &self.packages {
                let name = Self::crate_name(package);
                let runnable = scope.contains(package);
                let executable = deps.join(format!("{name}-0a1b"));
                if runnable {
                    fs::write(&executable, b"").unwrap();
                }
                message(serde_json::json!({
                    "reason": "compiler-artifact",
                    "package_id": format!("{package}-id"),
                    "target": {
                        "kind": ["lib"], "crate_types": ["lib"], "name": name,
                        "src_path": self.workspace.join("packages").join(package).join("src/lib.rs"),
                        "edition": "2024", "doc": true, "doctest": true, "test": true
                    },
                    "profile": {
                        "opt_level": "0", "debuginfo": 2, "debug_assertions": true,
                        "overflow_checks": true, "test": runnable
                    },
                    "features": [],
                    "filenames": [if runnable { executable.clone() } else { deps.join(format!("lib{name}-{SEED_HASH}.rlib")) }],
                    "executable": runnable.then_some(executable),
                    "fresh": false
                }));
            }
            message(serde_json::json!({"reason": "build-finished", "success": true}));
            bytes
        }

        /// Act as Cargo driving the task-private runner over the inventory.
        async fn run_tests(&self, invocation: &Invocation) -> Result<()> {
            let config = PathBuf::from(&invocation.args[1]);
            let runner: toml::Value = toml::from_str(&fs::read_to_string(&config)?)?;
            let command: Vec<_> = runner["target"][HOST_TRIPLE]["runner"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect();
            assert_eq!(command[1..3], ["coverage", "dispatch"]);
            let flag = |name: &str| {
                PathBuf::from(&command[command.iter().position(|arg| arg == name).unwrap() + 1])
            };
            let number = |name: &str| flag(name).to_str().unwrap().parse::<u32>().unwrap();
            let partition = PartitionScheme::new(number("--partition"), number("--partitions"))?;
            let inventory: Inventory = read_json(&flag("--inventory"))?;
            let target = flag("--target-dir");
            let instrumented = Self::env(invocation, "LLVM_PROFILE_FILE").is_some();
            let mut launcher = fixture::ScriptedLauncher {
                profiles: instrumented.then(|| target.clone()),
                ..Default::default()
            };
            for artifact in &inventory.artifacts {
                if let Some(executable) = &artifact.executable {
                    launcher.lists.insert(
                        target.join(executable),
                        (0..5)
                            .map(|test| format!("tests::case_{test}: test\n"))
                            .collect(),
                    );
                }
            }
            for artifact in inventory
                .artifacts
                .iter()
                .filter(|artifact| artifact.executable.is_some())
            {
                let executable = target.join(artifact.executable.as_deref().unwrap());
                dispatch_with(
                    &mut launcher,
                    &DispatchOptions {
                        root: &self.workspace,
                        inventory: &flag("--inventory"),
                        host: HOST_TRIPLE,
                        partition: &partition,
                        target_dir: &target,
                        ledger: &flag("--ledger"),
                        diagnostics: &flag("--diagnostics"),
                        deadline: flag("--deadline").to_str().unwrap().parse()?,
                        executable: &executable,
                        args: &[],
                    },
                    &self.workspace.join(&artifact.package_root),
                    &[],
                )
                .await?;
            }
            Ok(())
        }

        fn partition_of(target: &Path) -> PartitionScheme {
            let config = target.join(STATE).join("runner-config.toml");
            let text = fs::read_to_string(config).unwrap();
            let runner: toml::Value = toml::from_str(&text).unwrap();
            let command: Vec<_> = runner["target"][HOST_TRIPLE]["runner"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect();
            let number = |name: &str| {
                command[command.iter().position(|arg| arg == name).unwrap() + 1]
                    .parse()
                    .unwrap()
            };
            PartitionScheme::new(number("--partition"), number("--partitions")).unwrap()
        }
    }

    impl Host for Fake {
        async fn capture(&mut self, invocation: &Invocation) -> Result<String> {
            self.invocations.push(invocation.clone());
            let args: Vec<_> = invocation
                .args
                .iter()
                .map(|arg| arg.to_str().unwrap())
                .collect();
            if invocation.program == "mise" {
                self.call(Call::BinPaths)?;
                return Ok(self
                    .bin_paths
                    .clone()
                    .unwrap_or_else(|| self.llvm_cov_dir.display().to_string()));
            }
            if invocation.program == "rustc" {
                self.call(Call::Rustc)?;
                return Ok(self.rustc.clone());
            }
            match args[..] {
                ["llvm-cov", "--version"] => {
                    self.call(Call::Version)?;
                    Ok(self.version.clone())
                }
                ["llvm-cov", "show-env", "--pwsh"] => {
                    self.call(Call::ShowEnv)?;
                    if let Some(text) = &self.show_env {
                        return Ok(text.clone());
                    }
                    let target = Self::target(invocation);
                    let profile = target.join("kuru-%p-%4m.profraw");
                    let target = target.to_str().unwrap();
                    Ok([
                        pwsh("LLVM_PROFILE_FILE", profile.to_str().unwrap()),
                        pwsh("RUSTC_WRAPPER", "/fake/cargo-llvm-cov"),
                        pwsh(
                            "__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS",
                            "-C\u{1f}instrument-coverage",
                        ),
                        pwsh("CARGO_LLVM_COV_TARGET_DIR", target),
                    ]
                    .join("\n"))
                }
                _ => panic!("unexpected capture {}", invocation.describe()),
            }
        }

        async fn stream(&mut self, invocation: &Invocation, stdout: Option<&Path>) -> Result<()> {
            self.invocations.push(invocation.clone());
            let args: Vec<_> = invocation
                .args
                .iter()
                .map(|arg| arg.to_str().unwrap())
                .collect();
            match args[..] {
                ["metadata", ..] => {
                    self.call(Call::Metadata)?;
                    fs::write(stdout.unwrap(), serde_json::to_vec(&self.metadata())?)?;
                }
                ["test", .., "--no-run", _] => {
                    self.call(Call::NoRun)?;
                    let target = Self::target(invocation);
                    self.seeded_at_build = Some(
                        target
                            .join(format!("debug/build/cc-{SEED_HASH}/output"))
                            .is_file(),
                    );
                    if Self::env(invocation, "LLVM_PROFILE_FILE").is_some() {
                        // A compile-phase profile that must be discarded.
                        fs::write(target.join("kuru-0-0.profraw"), b"compile")?;
                    }
                    fs::write(stdout.unwrap(), self.messages(invocation))?;
                }
                ["run", "-p", "kuru-memory", ..] => {
                    assert!(stdout.is_none());
                    self.call(Call::Prefetch)?;
                    assert!(Self::env(invocation, "CARGO_TARGET_DIR").is_some());
                }
                ["--config", _, "test", ..] => {
                    assert!(stdout.is_none());
                    self.call(Call::TestRun)?;
                    self.run_tests(invocation).await?;
                }
                ["llvm-cov", "report", .., output] => {
                    assert!(stdout.is_none());
                    self.call(Call::Export)?;
                    let target = Self::target(invocation);
                    assert!(!target.join("kuru-0-0.profraw").exists());
                    if self.write_export {
                        let partition = Self::partition_of(&target);
                        let relative = fixture::lcov_for(&partition);
                        let absolute = relative.replace(
                            "SF:packages/",
                            &format!("SF:{}/packages/", self.workspace.display()),
                        );
                        fs::write(output, absolute)?;
                    }
                }
                _ => panic!("unexpected stream {}", invocation.describe()),
            }
            Ok(())
        }

        async fn verify_source(
            &mut self,
            root: &Path,
            source: &str,
            llvm_cov: Option<&Path>,
        ) -> Result<()> {
            assert_eq!(root, self.workspace);
            assert_eq!(source, fixture::SOURCE);
            if let Some(llvm_cov) = llvm_cov {
                assert!(llvm_cov.starts_with(&self.llvm_cov_dir));
            }
            self.verified.push(llvm_cov.map(Path::to_path_buf));
            self.call(Call::VerifySource)
        }

        async fn receipt(&mut self, options: &ReceiptOptions<'_>) -> Result<()> {
            self.call(Call::Receipt)?;
            assert!(!options.profiles.join("kuru-0-0.profraw").exists());
            self.receipts.push((
                options.mode,
                options.partition.clone(),
                options.run_attempt.to_owned(),
                options.output.to_owned(),
            ));
            super::super::write_evidence(options, fixture::identity(options.mode))
        }
    }

    struct Scenario {
        temp: TempDir,
        fake: Fake,
        helper: PathBuf,
        vars: BTreeMap<String, String>,
    }

    impl Scenario {
        fn new(step: Step) -> Self {
            let temp = TempDir::new().unwrap();
            let fake = Fake::new(temp.path());
            let helper = temp.path().join("kuru-delivery-helper");
            fs::write(&helper, b"").unwrap();
            let jobs = temp.path().join("job");
            let mut vars = BTreeMap::new();
            let mut set = |name: &str, value: String| {
                vars.insert(format!("{INPUT_PREFIX}{name}"), value);
            };
            set("SOURCE", fixture::SOURCE.to_owned());
            set("ATTEMPT", "2".to_owned());
            set("OS", super::super::LOCAL_OS.to_owned());
            set("PARTITIONS", "3".to_owned());
            match step {
                Step::Shard(mode) => {
                    set("TARGET", jobs.join("target").display().to_string());
                    set("PARTITION", "2".to_owned());
                    if mode == Mode::Uninstrumented {
                        set("PACKAGES", "kuru-memory, kuru-memory".to_owned());
                    }
                    set("OUTPUT", jobs.join("evidence").display().to_string());
                    set(
                        "DIAGNOSTICS",
                        jobs.join("diagnostics").display().to_string(),
                    );
                    set("JOB_STARTED", (unix_now().unwrap() - 60).to_string());
                    set("JOB_MINUTES", "45".to_owned());
                }
                Step::Merge => {
                    set("MODE", "instrumented".to_owned());
                    set("INPUTS", jobs.join("inputs").display().to_string());
                    set(
                        "REPORT",
                        jobs.join("report")
                            .join("coverage.lcov")
                            .display()
                            .to_string(),
                    );
                }
            }
            vars.insert("UNRELATED".to_owned(), "ignored".to_owned());
            vars.insert("RUST_TEST_THREADS".to_owned(), "2".to_owned());
            Self {
                temp,
                fake,
                helper,
                vars,
            }
        }

        fn set(&mut self, name: &str, value: &str) {
            self.vars
                .insert(format!("{INPUT_PREFIX}{name}"), value.to_owned());
        }

        /// A job path joined per component, as `std::path::absolute` spells it.
        fn job(&self, name: &str) -> PathBuf {
            name.split('/')
                .fold(self.temp.path().join("job"), |path, part| path.join(part))
        }

        async fn run(&mut self, step: Step) -> Result<()> {
            let vars: Vec<(OsString, OsString)> = self
                .vars
                .iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect();
            let root = self.fake.workspace.clone();
            run(step, &root, &self.helper, vars, &mut self.fake).await
        }

        /// Diagnostics other than the per-selection output logs every run keeps.
        fn diagnostics(&self) -> BTreeSet<String> {
            fs::read_dir(self.job("diagnostics"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .filter(|name| !name.ends_with(".stdout.log"))
                .collect()
        }

        fn failure(&self) -> String {
            fs::read_to_string(self.job("diagnostics").join("failure.txt")).unwrap()
        }

        fn state(&self) -> PathBuf {
            resolved_directory(&self.job("target")).unwrap().join(STATE)
        }

        fn job_ledger(&self) -> ledger::JobLedger {
            ledger::JobLedger::parse(&fs::read(self.state().join("job-ledger.json")).unwrap())
                .unwrap()
        }
    }

    const INSTRUMENTED: Step = Step::Shard(Mode::Instrumented);
    const UNINSTRUMENTED: Step = Step::Shard(Mode::Uninstrumented);

    #[tokio::test]
    async fn instrumented_partitions_sequence_inventory_runner_export_and_receipt() {
        let mut scenario = Scenario::new(INSTRUMENTED);
        scenario.run(INSTRUMENTED).await.unwrap();
        let fake = &scenario.fake;
        assert_eq!(
            fake.calls,
            [
                Call::BinPaths,
                Call::Version,
                Call::VerifySource,
                Call::ShowEnv,
                Call::Metadata,
                Call::NoRun,
                Call::Rustc,
                Call::TestRun,
                Call::Export,
                Call::Receipt,
            ]
        );
        let target = resolved_directory(&scenario.job("target")).unwrap();
        let state = scenario.state();
        for name in [
            "metadata.json",
            "full-messages.json",
            "inventory.json",
            "coverage.raw.lcov",
            "coverage.lcov",
        ] {
            assert!(state.join(name).is_file(), "{name}");
        }
        // Show-env ran with the target selected; every later Cargo child gets
        // show-env's own profile path, applied after the preset one.
        let show_env = &fake.invocations[2];
        assert_eq!(
            Fake::env(show_env, "LLVM_PROFILE_FILE").unwrap(),
            target.join("kuru-%p-%m.profraw").as_os_str()
        );
        let cargo: Vec<_> = fake
            .invocations
            .iter()
            .filter(|invocation| invocation.program == "cargo")
            .collect();
        assert_eq!(cargo.len(), 3);
        for invocation in &cargo {
            assert_eq!(invocation.cwd, fake.workspace);
            assert_eq!(
                Fake::env(invocation, "LLVM_PROFILE_FILE").unwrap(),
                target.join("kuru-%p-%4m.profraw").as_os_str()
            );
            assert_eq!(
                Fake::env(invocation, "RUSTC_WRAPPER").unwrap(),
                "/fake/cargo-llvm-cov"
            );
            let names: BTreeSet<_> = invocation.env.iter().map(|(key, _)| key).collect();
            assert_eq!(names.len(), invocation.env.len(), "duplicate child env");
        }
        assert_eq!(
            cargo[2].args,
            [
                "--config",
                state.join("runner-config.toml").to_str().unwrap(),
                "test",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
                "--no-fail-fast",
            ]
            .map(OsString::from)
        );
        let export = fake
            .invocations
            .iter()
            .find(|invocation| {
                invocation.args.first().is_some_and(|arg| arg == "llvm-cov")
                    && invocation.args.get(1).is_some_and(|arg| arg == "report")
            })
            .unwrap();
        assert!(!export.args.iter().any(|arg| arg == "--fail-under-lines"));
        assert!(Fake::env(export, "CARGO_LLVM_COV_TARGET_DIR").is_some());
        // Captures that are not Cargo children never see the coverage env.
        for invocation in fake
            .invocations
            .iter()
            .filter(|invocation| invocation.program == "mise" || invocation.program == "rustc")
        {
            assert!(invocation.env.is_empty(), "{}", invocation.describe());
        }
        let (mode, partition, attempt, output) = &fake.receipts[0];
        assert_eq!(*mode, Mode::Instrumented);
        assert_eq!(*partition, PartitionScheme::new(2, 3).unwrap());
        assert_eq!(attempt, "2");
        assert_eq!(*output, scenario.job("evidence"));
        let evidence = scenario.job("evidence/attempt-2");
        let receipt: super::super::Receipt = read_json(&evidence.join("receipt.json")).unwrap();
        assert_eq!(receipt.scope, WORKSPACE_PACKAGES);
        let lcov = fs::read_to_string(evidence.join("coverage.lcov")).unwrap();
        assert!(
            lcov.starts_with("SF:packages/kuru-core/src/lib.rs\n"),
            "{lcov}"
        );
        let ledger = scenario.job_ledger();
        for phase in ["prepare", "seed_import", "compile", "tests", "lcov_export"] {
            assert!(ledger.phases.contains_key(phase), "{phase}");
        }
        assert_eq!(ledger.cache.helper, "unknown");
        assert_eq!(ledger.cache.seed, "absent");
        assert_eq!(ledger.cache.dependency_units_rebuilt, 1);
        assert_eq!(ledger.cache.workspace_units_rebuilt, 8);
        assert_eq!(ledger.executables.len(), 8);
        assert!(ledger.profiles.count > 0);
        assert!(scenario.diagnostics().is_empty());
    }

    #[tokio::test]
    async fn uninstrumented_partitions_prefetch_and_never_instrument() {
        let mut scenario = Scenario::new(UNINSTRUMENTED);
        scenario.run(UNINSTRUMENTED).await.unwrap();
        let fake = &scenario.fake;
        assert_eq!(
            fake.calls,
            [
                Call::VerifySource,
                Call::Metadata,
                Call::NoRun,
                Call::Rustc,
                Call::Prefetch,
                Call::TestRun,
                Call::Receipt,
            ]
        );
        assert_eq!(fake.verified, [None]);
        for invocation in &fake.invocations {
            assert!(Fake::env(invocation, "LLVM_PROFILE_FILE").is_none());
            assert!(invocation.program != "mise");
        }
        let build = fake
            .invocations
            .iter()
            .find(|invocation| invocation.args.iter().any(|arg| arg == "--no-run"))
            .unwrap();
        assert_eq!(
            build.args[..3],
            ["test", "-p", "kuru-memory"].map(OsString::from)
        );
        let evidence = scenario.job("evidence/attempt-2");
        assert!(!evidence.join("coverage.lcov").exists());
        let receipt: super::super::Receipt = read_json(&evidence.join("receipt.json")).unwrap();
        assert_eq!(receipt.mode, Mode::Uninstrumented);
        assert_eq!(receipt.scope, ["kuru-memory"]);
        assert_eq!(receipt.cargo_llvm_cov, None);
        assert!(scenario.job_ledger().phases.contains_key("prefetch"));
    }

    /// Run every partition of one OS in its own workspace and target, place
    /// each evidence artifact as the download step would, and merge.
    async fn partitions(step: Step, count: u32, skip: Option<u32>) -> Scenario {
        let mut merge = Scenario::new(Step::Merge);
        merge.set("PARTITIONS", &count.to_string());
        let Step::Shard(mode) = step else {
            unreachable!()
        };
        merge.set("MODE", mode.name());
        if mode == Mode::Uninstrumented {
            merge.set("PACKAGES", "kuru-memory");
        }
        let inputs = merge.job("inputs");
        fs::create_dir_all(&inputs).unwrap();
        for index in (1..=count).filter(|index| Some(*index) != skip) {
            let mut partition = Scenario::new(step);
            partition.set("PARTITION", &index.to_string());
            partition.set("PARTITIONS", &count.to_string());
            partition.run(step).await.unwrap();
            fs::rename(
                partition.job("evidence"),
                inputs.join(format!("ci-coverage-local-partition-{index}-attempt-2")),
            )
            .unwrap();
        }
        merge
    }

    #[tokio::test]
    async fn independent_partitions_merge_into_one_gated_report() {
        let mut merge = partitions(INSTRUMENTED, 3, None).await;
        merge.run(Step::Merge).await.unwrap();
        let report = fs::read_to_string(merge.job("report/coverage.lcov")).unwrap();
        assert!(report.contains("LF:20\nLH:19\n"), "{report}");
        let summary: merge::MergeSummary =
            read_json(&merge.job(&format!("report/{}", merge::SUMMARY_FILE))).unwrap();
        assert_eq!(summary.partitions.len(), 3);
        assert_eq!(summary.executables, 8);
        assert_eq!(summary.tests, 40);
        assert!(merge.fake.calls.is_empty(), "the merge starts no process");

        let mut memory = partitions(UNINSTRUMENTED, 3, None).await;
        memory.run(Step::Merge).await.unwrap();
        assert!(!memory.job("report/coverage.lcov").exists());
        assert!(
            memory
                .job(&format!("report/{}", merge::SUMMARY_FILE))
                .is_file()
        );

        // A missing partition fails the merge before any report exists.
        let mut missing = partitions(INSTRUMENTED, 3, Some(3)).await;
        let error = format!("{:#}", missing.run(Step::Merge).await.unwrap_err());
        assert!(
            error.contains("partition 3 of 3 has no evidence"),
            "{error}"
        );
        assert!(
            !missing.job("report").exists()
                || fs::read_dir(missing.job("report"))
                    .unwrap()
                    .next()
                    .is_none()
        );
        // The mode of the evidence must be the merge's.
        let mut mode = partitions(UNINSTRUMENTED, 1, None).await;
        mode.set("MODE", "instrumented");
        mode.vars.remove(&format!("{INPUT_PREFIX}PACKAGES"));
        let error = format!("{:#}", mode.run(Step::Merge).await.unwrap_err());
        assert!(
            error.contains("unexpected artifact entries") || error.contains("is uninstrumented"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn inputs_are_required_per_step_before_any_effect() {
        let mut shard = Scenario::new(INSTRUMENTED);
        for name in ["PARTITION", "JOB_MINUTES", "OS"] {
            shard.vars.remove(&format!("{INPUT_PREFIX}{name}"));
        }
        shard.set("SEED", "  ");
        let error = shard.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert_eq!(
            error,
            "coverage shard requires KURU_COVERAGE_OS, KURU_COVERAGE_PARTITION, \
             KURU_COVERAGE_JOB_MINUTES"
        );
        assert!(shard.fake.calls.is_empty());
        assert!(!shard.job("diagnostics").exists());

        let mut memory = Scenario::new(UNINSTRUMENTED);
        memory.vars.remove(&format!("{INPUT_PREFIX}PACKAGES"));
        let error = memory.run(UNINSTRUMENTED).await.unwrap_err().to_string();
        assert_eq!(error, "coverage shard requires KURU_COVERAGE_PACKAGES");

        let mut packages = Scenario::new(INSTRUMENTED);
        packages.set("PACKAGES", "kuru-core");
        let error = packages.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(error.contains("unset KURU_COVERAGE_PACKAGES"), "{error}");

        let mut merge = Scenario::new(Step::Merge);
        merge.vars.remove(&format!("{INPUT_PREFIX}REPORT"));
        let error = merge.run(Step::Merge).await.unwrap_err().to_string();
        assert_eq!(error, "coverage merge requires KURU_COVERAGE_REPORT");
        // An uninstrumented merge pins the package scope it expects.
        let mut scope = Scenario::new(Step::Merge);
        scope.set("MODE", "uninstrumented");
        let error = scope.run(Step::Merge).await.unwrap_err().to_string();
        assert_eq!(error, "uninstrumented mode requires KURU_COVERAGE_PACKAGES");
        let mut mode = Scenario::new(Step::Merge);
        mode.set("MODE", "partial");
        let error = mode.run(Step::Merge).await.unwrap_err().to_string();
        assert!(error.contains("instrumented or uninstrumented"), "{error}");

        let mut minutes = Scenario::new(INSTRUMENTED);
        minutes.set("JOB_MINUTES", "ninety");
        let error = minutes.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(error.contains("JOB_MINUTES must be"), "{error}");
        assert!(!minutes.job("diagnostics").exists());

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let error = Inputs::parse(
                Step::Merge,
                [(
                    OsString::from("KURU_COVERAGE_TARGET"),
                    OsString::from_vec(vec![0xff]),
                )],
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("KURU_COVERAGE_TARGET is not UTF-8"),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn existing_directories_are_refused() {
        // Diagnostics that already exist are neither reused nor written.
        let mut diagnostics = Scenario::new(INSTRUMENTED);
        fs::create_dir_all(diagnostics.job("diagnostics")).unwrap();
        let error = diagnostics.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(
            error.contains("coverage diagnostics already exists"),
            "{error}"
        );
        assert!(diagnostics.diagnostics().is_empty());
        assert!(diagnostics.fake.calls.is_empty());

        // An existing target is refused inside the diagnostics guard.
        let mut target = Scenario::new(INSTRUMENTED);
        fs::create_dir_all(target.job("target")).unwrap();
        let error = target.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(error.contains("coverage target already exists"), "{error}");
        assert_eq!(
            target.diagnostics(),
            BTreeSet::from(["failure.txt".to_owned()])
        );
        assert!(target.failure().contains("coverage target already exists"));
        assert!(target.fake.calls.is_empty());

        // An existing seed export is refused before anything is built.
        let mut export = Scenario::new(INSTRUMENTED);
        export.set("PARTITION", "1");
        let destination = export.job("seed-export");
        fs::create_dir_all(&destination).unwrap();
        export.set("SEED_EXPORT", destination.to_str().unwrap());
        let error = export.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(error.contains("seed export already exists"), "{error}");
        assert!(export.fake.calls.is_empty());

        // An existing report is refused before any evidence is read.
        let mut report = Scenario::new(Step::Merge);
        fs::create_dir_all(report.job("report")).unwrap();
        fs::write(report.job("report/coverage.lcov"), b"old").unwrap();
        let error = format!("{:#}", report.run(Step::Merge).await.unwrap_err());
        assert!(error.contains("coverage report already exists"), "{error}");
        assert_eq!(
            fs::read(report.job("report/coverage.lcov")).unwrap(),
            b"old"
        );
    }

    #[tokio::test]
    async fn partition_inputs_are_checked_before_any_effect() {
        for (name, value, reason) in [
            ("PARTITION", "0", "partition index 0"),
            ("PARTITION", "4", "partition index 4"),
            ("OS", "ubuntu-latest", "partitions number 8, not 3"),
            ("OS", "Ubuntu", "[a-z0-9]"),
            ("ATTEMPT", "02", "run attempt"),
            ("PARTITIONS", "99", "outside 1..=64"),
        ] {
            let mut scenario = Scenario::new(INSTRUMENTED);
            scenario.set(name, value);
            let error = scenario.run(INSTRUMENTED).await.unwrap_err().to_string();
            assert!(error.contains(reason), "{name}={value}: {error}");
            assert!(scenario.failure().contains(reason));
            assert!(scenario.fake.calls.is_empty());
            assert!(!scenario.job("target").exists());
        }
        let mut unknown = Scenario::new(UNINSTRUMENTED);
        unknown.set("PACKAGES", "kuru-memory,kuru-nope");
        let error = unknown.run(UNINSTRUMENTED).await.unwrap_err().to_string();
        assert!(
            error.contains("unknown coverage package kuru-nope"),
            "{error}"
        );
        let mut export = Scenario::new(INSTRUMENTED);
        export.set("SEED_EXPORT", export.job("seed").to_str().unwrap());
        let error = export.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(error.contains("only partition 1 exports"), "{error}");
        let mut helper = Scenario::new(INSTRUMENTED);
        helper.helper = helper.temp.path().join("missing-helper");
        let error = helper.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(error.contains("coverage helper"), "{error}");
        let mut merge = Scenario::new(Step::Merge);
        merge.set("OS", "Ubuntu");
        let error = merge.run(Step::Merge).await.unwrap_err().to_string();
        assert!(error.contains("[a-z0-9]"), "{error}");
    }

    #[tokio::test]
    async fn every_partition_failure_leaves_its_diagnostics() {
        let early = BTreeSet::from(["failure.txt".to_owned()]);
        let manifests = BTreeSet::from(
            ["failure.txt", "inventory.json", "runner-config.toml"].map(str::to_owned),
        );
        let mut ledger = manifests.clone();
        ledger.insert("runner-ledger.jsonl".to_owned());
        let mut job = ledger.clone();
        job.insert("job-ledger.json".to_owned());
        for (call, expected) in [
            (Call::BinPaths, &early),
            (Call::Version, &early),
            (Call::VerifySource, &early),
            (Call::ShowEnv, &early),
            (Call::Metadata, &early),
            (Call::NoRun, &early),
            (Call::TestRun, &manifests),
            (Call::Export, &ledger),
            (Call::Receipt, &job),
        ] {
            let mut scenario = Scenario::new(INSTRUMENTED);
            scenario.fake.fail = Some(call);
            let error = format!("{:#}", scenario.run(INSTRUMENTED).await.unwrap_err());
            let injected = format!("injected {call:?} failure");
            assert!(error.contains(&injected), "{call:?}: {error}");
            assert_eq!(&scenario.diagnostics(), expected, "{call:?}");
            assert!(scenario.failure().contains(&injected), "{call:?}");
            assert_eq!(scenario.fake.calls.last(), Some(&call));
        }
        let mut prefetch = Scenario::new(UNINSTRUMENTED);
        prefetch.fake.fail = Some(Call::Prefetch);
        let error = format!("{:#}", prefetch.run(UNINSTRUMENTED).await.unwrap_err());
        assert!(error.contains("memory supervisor snapshot"), "{error}");
        let mut rustc = Scenario::new(INSTRUMENTED);
        rustc.fake.rustc = "rustc 1.98.1".to_owned();
        let error = rustc.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(error.contains("one host target"), "{error}");
        assert!(rustc.diagnostics().contains("inventory.json"));
        let mut export = Scenario::new(INSTRUMENTED);
        export.fake.write_export = false;
        let error = format!("{:#}", export.run(INSTRUMENTED).await.unwrap_err());
        assert!(error.contains("export was not written"), "{error}");
        let mut drift = Scenario::new(INSTRUMENTED);
        drift.fake.packages.pop();
        let error = drift.run(INSTRUMENTED).await.unwrap_err().to_string();
        assert!(error.contains("differ from WORKSPACE_PACKAGES"), "{error}");
    }

    #[tokio::test]
    async fn toolchain_and_coverage_environment_are_checked_before_cargo() {
        type Setup = fn(&mut Fake);
        let cases: [(Setup, &str); 7] = [
            (
                |fake| fake.version = "cargo-llvm-cov 0.9.0".to_owned(),
                "expected cargo-llvm-cov 0.9.1, got cargo-llvm-cov 0.9.0",
            ),
            (
                |fake| fake.bin_paths = Some(String::new()),
                "did not name one aqua:taiki-e/cargo-llvm-cov@0.9.1 directory",
            ),
            (
                |fake| fake.bin_paths = Some("/a\n/b".to_owned()),
                "did not name one aqua:taiki-e/cargo-llvm-cov@0.9.1 directory",
            ),
            (
                |fake| fake.bin_paths = Some("/nonexistent-kuru-bin".to_owned()),
                "pinned cargo-llvm-cov executable is missing",
            ),
            (
                |fake| fake.show_env = Some("export RUSTC_WRAPPER=x".to_owned()),
                "unexpected show-env line 1",
            ),
            (
                |fake| fake.show_env = Some(pwsh("CARGO_LLVM_COV_TARGET_DIR", "/elsewhere")),
                "cargo-llvm-cov selected target /elsewhere",
            ),
            (
                |fake| fake.show_env = Some(pwsh("CARGO_LLVM_COV", "1")),
                "omits CARGO_LLVM_COV_TARGET_DIR",
            ),
        ];
        for (setup, reason) in cases {
            let mut scenario = Scenario::new(INSTRUMENTED);
            setup(&mut scenario.fake);
            let error = format!("{:#}", scenario.run(INSTRUMENTED).await.unwrap_err());
            assert!(error.contains(reason), "{reason}: {error}");
            assert!(
                !scenario.fake.calls.contains(&Call::Metadata),
                "{reason}: Cargo ran"
            );
            assert_eq!(
                scenario.diagnostics(),
                BTreeSet::from(["failure.txt".to_owned()])
            );
        }
    }

    #[tokio::test]
    async fn seeds_are_imported_before_the_build_and_exported_after_the_receipt() {
        let mut scenario = Scenario::new(INSTRUMENTED);
        scenario.set("PARTITION", "1");
        let seed = scenario.job("seed");
        let entry = seed.join(format!("debug/build/cc-{SEED_HASH}"));
        fs::create_dir_all(&entry).unwrap();
        fs::write(entry.join("output"), b"cc").unwrap();
        let workspace_entry = seed.join(format!("debug/.fingerprint/kuru-core-{SEED_HASH}"));
        fs::create_dir_all(&workspace_entry).unwrap();
        fs::write(workspace_entry.join("lib"), b"w").unwrap();
        scenario.set("SEED", seed.to_str().unwrap());
        scenario.set("SEED_CACHE", "true");
        scenario.set(
            "SEED_MATCHED_KEY",
            "kuru-coverage-seed-v1-local-instrumented-x",
        );
        scenario.set("HELPER_CACHE", "false");
        let export = scenario.job("seed-export");
        scenario.set("SEED_EXPORT", export.to_str().unwrap());
        scenario.run(INSTRUMENTED).await.unwrap();
        assert_eq!(scenario.fake.seeded_at_build, Some(true));
        let ledger = scenario.job_ledger();
        assert_eq!(ledger.cache.seed, "hit");
        assert_eq!(ledger.cache.helper, "miss");
        assert_eq!(
            ledger.cache.seed_matched_key.as_deref(),
            Some("kuru-coverage-seed-v1-local-instrumented-x")
        );
        assert_eq!(ledger.cache.seed_imported.entries, 1);
        assert_eq!(ledger.cache.seed_refused.get("workspace"), Some(&1));
        // The export holds the imported and built dependencies, never the
        // workspace library the build left beside them.
        assert!(
            export
                .join(format!("debug/build/cc-{SEED_HASH}/output"))
                .is_file()
        );
        assert!(
            export
                .join(format!("debug/deps/libserde-{SEED_HASH}.rlib"))
                .is_file()
        );
        assert!(
            !export
                .join(format!("debug/deps/libkuru_core-{SEED_HASH}.rlib"))
                .exists()
        );

        // An evicted seed is a miss and the partition still succeeds.
        let mut evicted = Scenario::new(INSTRUMENTED);
        evicted.set("SEED", evicted.job("absent").to_str().unwrap());
        evicted.set("SEED_CACHE", "false");
        evicted.run(INSTRUMENTED).await.unwrap();
        assert_eq!(evicted.job_ledger().cache.seed, "miss");
        assert_eq!(evicted.fake.seeded_at_build, Some(false));
    }

    #[test]
    fn profile_environment_digests_ignore_target_paths_only() {
        let env = |threads: Option<&str>| -> BTreeMap<String, Option<String>> {
            PROFILE_ENV
                .iter()
                .map(|name| {
                    (
                        (*name).to_owned(),
                        (*name == "RUST_TEST_THREADS")
                            .then(|| threads.map(str::to_owned))
                            .flatten(),
                    )
                })
                .collect()
        };
        let coverage = |target: &str| {
            vec![
                (
                    "LLVM_PROFILE_FILE".to_owned(),
                    format!("{target}/kuru-%p-%4m.profraw"),
                ),
                ("CARGO_LLVM_COV_TARGET_DIR".to_owned(), target.to_owned()),
            ]
        };
        let first = profile_env_sha256(&env(Some("2")), &coverage("/a/t1"), &["/a/t1"]).unwrap();
        let second =
            profile_env_sha256(&env(Some("2")), &coverage("/b/t2"), &["/b/t2", ""]).unwrap();
        assert_eq!(first, second);
        let threads = profile_env_sha256(&env(Some("4")), &coverage("/a/t1"), &["/a/t1"]).unwrap();
        assert_ne!(first, threads);
        let unset = profile_env_sha256(&env(None), &[], &[]).unwrap();
        assert_ne!(unset, profile_env_sha256(&env(Some("")), &[], &[]).unwrap());
        assert_eq!(
            package_args(Mode::Instrumented, &["x".to_owned()]),
            ["--workspace"]
        );
        assert_eq!(
            package_args(Mode::Uninstrumented, &["a".to_owned(), "b".to_owned()]),
            ["-p", "a", "-p", "b"]
        );
    }

    #[test]
    fn invocations_describe_their_command_and_keep_env_separate() {
        let invocation = Invocation::new("cargo", &["metadata", "--locked"], Path::new("/r"))
            .with_env(&[("A".into(), "1".into())]);
        assert_eq!(invocation.describe(), "cargo metadata --locked");
        assert_eq!(invocation.env, [(OsString::from("A"), OsString::from("1"))]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn system_host_streams_and_captures_real_children() {
        let temp = TempDir::new().unwrap();
        let mut host = System;
        let echo = Invocation::new(
            "/bin/sh",
            &["-c", "printf '%s' \"$KURU_ORCHESTRATE_PROBE\""],
            temp.path(),
        )
        .with_env(&[("KURU_ORCHESTRATE_PROBE".into(), " value ".into())]);
        assert_eq!(host.capture(&echo).await.unwrap(), "value");
        let output = temp.path().join("streamed.json");
        host.stream(&echo, Some(&output)).await.unwrap();
        assert_eq!(fs::read(&output).unwrap(), b" value ");
        // The stream never overwrites an existing file.
        let error = format!("{:#}", host.stream(&echo, Some(&output)).await.unwrap_err());
        assert!(error.contains("create"), "{error}");
        host.stream(
            &Invocation::new("/bin/sh", &["-c", "exit 0"], temp.path()),
            None,
        )
        .await
        .unwrap();
        let failing = Invocation::new("/bin/sh", &["-c", "echo nope >&2; exit 4"], temp.path());
        let error = host.capture(&failing).await.unwrap_err().to_string();
        assert!(
            error.contains("failed with") && error.contains("nope"),
            "{error}"
        );
        let error = host.stream(&failing, None).await.unwrap_err().to_string();
        assert!(
            error.contains("/bin/sh -c echo nope >&2; exit 4 failed"),
            "{error}"
        );
        let missing = Invocation::new("/nonexistent/kuru-probe", &[], temp.path());
        assert!(host.capture(&missing).await.is_err());
        assert!(host.stream(&missing, None).await.is_err());
        let binary = Invocation::new("/bin/sh", &["-c", "printf '\\377'"], temp.path());
        let error = host.capture(&binary).await.unwrap_err().to_string();
        assert!(error.contains("non-UTF-8"), "{error}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn system_host_streams_and_captures_real_windows_children() {
        let temp = TempDir::new().unwrap();
        let mut host = System;
        let echo = Invocation::new("cmd.exe", &["/c", "echo value"], temp.path());
        assert_eq!(host.capture(&echo).await.unwrap(), "value");
        let output = temp.path().join("streamed.txt");
        host.stream(&echo, Some(&output)).await.unwrap();
        assert_eq!(fs::read_to_string(&output).unwrap().trim(), "value");
        // The stream never overwrites an existing file.
        assert!(host.stream(&echo, Some(&output)).await.is_err());
        host.stream(&echo, None).await.unwrap();
        let failing = Invocation::new("cmd.exe", &["/c", "exit 4"], temp.path());
        let error = host.capture(&failing).await.unwrap_err().to_string();
        assert!(error.contains("failed with"), "{error}");
        let error = host.stream(&failing, None).await.unwrap_err().to_string();
        assert!(error.contains("failed with"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn failure_recording_reports_but_never_masks_the_error() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let diagnostics = temp.path().join("diagnostics");
        let state = temp.path().join("state");
        fs::create_dir(&diagnostics).unwrap();
        fs::create_dir(&state).unwrap();
        fs::write(state.join("inventory.json"), b"{}").unwrap();
        // A directory in place of a state file is never copied.
        fs::create_dir(state.join("runner-config.toml")).unwrap();
        let error = anyhow::anyhow!("inner").context("outer");
        record_failure(&diagnostics, Some(&state), &error);
        let failure = fs::read_to_string(diagnostics.join("failure.txt")).unwrap();
        assert!(
            failure.contains("outer") && failure.contains("inner"),
            "{failure}"
        );
        assert!(diagnostics.join("inventory.json").is_file());
        assert!(!diagnostics.join("runner-config.toml").exists());

        // Unwritable diagnostics are reported to stderr without panicking.
        let locked = temp.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        record_failure(&locked, Some(&state), &error);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    }
}
