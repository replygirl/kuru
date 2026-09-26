//! One cross-platform orchestrator for a coverage shard and its per-OS report.
//!
//! `coverage shard` compiles the full instrumented workspace inventory into a
//! fresh target, runs it through the task-private Cargo runner that executes
//! only the shard's packages, and writes its receipt. `coverage collect`
//! rebuilds the same inventory, accepts each shard's latest receipt for this
//! OS and enforces the single 90% workspace line report.
//!
//! Every process this module starts goes through [`Host`], so the sequencing
//! and each refusal are unit-tested with a fake. The coverage environment from
//! `cargo-llvm-cov show-env` is applied only to those children: the helper's
//! own process, which is also Cargo's runner, is never instrumented.

use super::{
    COMMAND_OUTPUT_LIMIT, CollectOptions, LLVM_COV_VERSION, ReceiptOptions, RunnerConfigOptions,
    artifact_os_label, canonical_attempt, discard_compile_profiles, shard_packages,
    validate_run_ledger, workspace_packages, write_inventory, write_runner_config, write_selection,
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
const LLVM_COV_TOOL: &str = "cargo:cargo-llvm-cov@0.9.1";
/// Bound for short captured commands. `show-env` resolves full Cargo metadata,
/// which may fetch dependency manifests on a cold runner.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Private state directory created inside the fresh coverage target.
const STATE: &str = "kuru-shard-state";
/// State copied beside `failure.txt` when a shard fails after creating it.
const DIAGNOSTIC_COPIES: [&str; 4] = [
    "inventory.json",
    "selection.json",
    "runner-config.toml",
    "runner-ledger.jsonl",
];
const INPUT_PREFIX: &str = "KURU_COVERAGE_";

/// Run one coverage shard from the process's `KURU_COVERAGE_*` inputs.
pub async fn shard(root: &Path) -> Result<()> {
    // `cargo run` built this helper in the ordinary target before any coverage
    // environment existed; it is the Cargo runner and is never rebuilt here.
    let helper = std::env::current_exe().context("locate the coverage helper")?;
    run(Mode::Shard, root, &helper, std::env::vars_os(), &mut System).await
}

/// Collect every shard receipt for this OS and enforce the workspace report.
pub async fn collect(root: &Path) -> Result<()> {
    let helper = std::env::current_exe().context("locate the coverage helper")?;
    run(
        Mode::Collect,
        root,
        &helper,
        std::env::vars_os(),
        &mut System,
    )
    .await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Shard,
    Collect,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Shard => "shard",
            Self::Collect => "collect",
        }
    }

    fn required(self) -> &'static [&'static str] {
        match self {
            Self::Shard => &[
                "TARGET",
                "SOURCE",
                "ATTEMPT",
                "OS",
                "SHARD",
                "PACKAGES",
                "OUTPUT",
                "DIAGNOSTICS",
                "JOB_STARTED",
                "JOB_MINUTES",
            ],
            Self::Collect => &["TARGET", "SOURCE", "ATTEMPT", "OS", "INPUTS", "REPORT"],
        }
    }
}

#[derive(Debug)]
struct Common {
    target: PathBuf,
    source: String,
    attempt: String,
    os: String,
}

#[derive(Debug)]
struct ShardInputs {
    shard: String,
    packages: Vec<String>,
    output: PathBuf,
    diagnostics: PathBuf,
    job_started: u64,
    job_minutes: u64,
}

#[derive(Debug)]
struct CollectInputs {
    inputs: PathBuf,
    report: PathBuf,
}

#[derive(Debug)]
enum Plan {
    Shard(ShardInputs),
    Collect(CollectInputs),
}

#[derive(Debug)]
struct Inputs {
    common: Common,
    plan: Plan,
}

impl Inputs {
    /// Require every input of the mode to be present and well formed. Values
    /// are read from the supplied environment, never the process's own.
    fn parse(mode: Mode, vars: impl IntoIterator<Item = (OsString, OsString)>) -> Result<Self> {
        let mut values = BTreeMap::new();
        for (key, value) in vars {
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
        let missing: Vec<_> = mode
            .required()
            .iter()
            .filter(|name| !values.contains_key(**name))
            .map(|name| format!("{INPUT_PREFIX}{name}"))
            .collect();
        ensure!(
            missing.is_empty(),
            "coverage {} requires {}",
            mode.name(),
            missing.join(", ")
        );
        let text = |name: &str| values[name].clone();
        let path = |name: &str| {
            std::path::absolute(&values[name])
                .with_context(|| format!("resolve {INPUT_PREFIX}{name}"))
        };
        let number = |name: &str| {
            values[name]
                .trim()
                .parse::<u64>()
                .with_context(|| format!("{INPUT_PREFIX}{name} must be a non-negative integer"))
        };
        let common = Common {
            target: path("TARGET")?,
            source: text("SOURCE"),
            attempt: text("ATTEMPT"),
            os: text("OS"),
        };
        let plan = match mode {
            Mode::Shard => {
                let mut packages: Vec<_> = values["PACKAGES"]
                    .split(',')
                    .map(str::trim)
                    .filter(|package| !package.is_empty())
                    .map(str::to_owned)
                    .collect();
                packages.sort();
                packages.dedup();
                Plan::Shard(ShardInputs {
                    shard: text("SHARD"),
                    packages,
                    output: path("OUTPUT")?,
                    diagnostics: path("DIAGNOSTICS")?,
                    job_started: number("JOB_STARTED")?,
                    job_minutes: number("JOB_MINUTES")?,
                })
            }
            Mode::Collect => Plan::Collect(CollectInputs {
                inputs: path("INPUTS")?,
                report: path("REPORT")?,
            }),
        };
        Ok(Self { common, plan })
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
    /// inherited. Compilation and the test run are not under the shard
    /// deadline: the Cargo runner enforces that per test executable.
    async fn stream(&mut self, invocation: &Invocation, stdout: Option<&Path>) -> Result<()>;
    /// Reject a different or modified tracked source (git, rustc, cargo).
    async fn verify_source(&mut self, root: &Path, source: &str, llvm_cov: &Path) -> Result<()>;
    /// Bind the shard's profiles to its exact identity (git, rustc, cargo).
    async fn receipt(&mut self, options: &ReceiptOptions<'_>) -> Result<()>;
    /// Accept each shard's latest receipt and copy its profiles.
    async fn collect(&mut self, options: &CollectOptions<'_>) -> Result<Vec<(String, u64)>>;
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

    async fn verify_source(&mut self, root: &Path, source: &str, llvm_cov: &Path) -> Result<()> {
        super::verify_source(root, source, llvm_cov).await
    }

    async fn receipt(&mut self, options: &ReceiptOptions<'_>) -> Result<()> {
        super::write_receipt(options).await
    }

    async fn collect(&mut self, options: &CollectOptions<'_>) -> Result<Vec<(String, u64)>> {
        super::collect_profiles(options).await
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
    mode: Mode,
    root: &Path,
    helper: &Path,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    host: &mut H,
) -> Result<()> {
    let Inputs { common, plan } = Inputs::parse(mode, vars)?;
    match plan {
        Plan::Shard(inputs) => {
            // Every later failure, including toolchain and inventory setup,
            // leaves its error and the shard state beside the per-test logs.
            create_fresh_directory(&inputs.diagnostics, "coverage diagnostics")?;
            let diagnostics = resolved_directory(&inputs.diagnostics)?;
            let mut state = None;
            let result = run_shard(
                host,
                root,
                helper,
                &common,
                &inputs,
                &diagnostics,
                &mut state,
            )
            .await;
            if let Err(error) = &result {
                record_failure(&diagnostics, state.as_deref(), error);
            }
            result
        }
        Plan::Collect(inputs) => run_collect(host, root, &common, &inputs).await,
    }
}

/// Write `failure.txt` and copy the shard's manifests and ledger. A failure
/// to record diagnostics is reported but never replaces the shard's error.
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

/// The prepared full instrumented inventory shared by shard and collect.
struct Prepared {
    root: PathBuf,
    target: PathBuf,
    state: PathBuf,
    llvm_cov: PathBuf,
    env: Vec<(OsString, OsString)>,
    inventory: PathBuf,
}

async fn prepare<H: Host>(
    host: &mut H,
    root: &Path,
    common: &Common,
    state_slot: &mut Option<PathBuf>,
) -> Result<Prepared> {
    let root = resolved_directory(root).context("coverage root")?;
    // One coverage writer per target: a shard never reuses an existing tree.
    create_fresh_directory(&common.target, "coverage target")?;
    let target = resolved_directory(&common.target)?;
    let state = target.join(STATE);
    fs::create_dir(&state).with_context(|| format!("create {}", state.display()))?;
    *state_slot = Some(state.clone());

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
    let llvm_cov =
        Path::new(bin.trim()).join(format!("cargo-llvm-cov{}", std::env::consts::EXE_SUFFIX));
    ensure!(
        llvm_cov.is_absolute() && fs::metadata(&llvm_cov).is_ok_and(|metadata| metadata.is_file()),
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
    host.verify_source(&root, &common.source, &llvm_cov).await?;

    let mut env = vec![
        (OsString::from("CARGO_TARGET_DIR"), target.clone().into()),
        (
            OsString::from("CARGO_LLVM_COV_TARGET_DIR"),
            target.clone().into(),
        ),
        (
            OsString::from("LLVM_PROFILE_FILE"),
            target.join("kuru-%p-%m.profraw").into(),
        ),
    ];
    let shown = host
        .capture(
            &Invocation::new(&llvm_cov, &["llvm-cov", "show-env", "--pwsh"], &root).with_env(&env),
        )
        .await?;
    let coverage =
        parse_show_env(&shown).context("apply the cargo-llvm-cov coverage environment")?;
    let shown_target = coverage
        .iter()
        .find_map(|(name, value)| (name == "CARGO_LLVM_COV_TARGET_DIR").then_some(value))
        .context("cargo-llvm-cov show-env omits CARGO_LLVM_COV_TARGET_DIR")?;
    ensure!(
        Path::new(shown_target) == target,
        "cargo-llvm-cov selected target {shown_target}, expected {}",
        target.display()
    );
    // The coverage environment is applied last, as a shell evaluating
    // `show-env` would: its LLVM_PROFILE_FILE names this target's profiles.
    for (name, value) in coverage {
        env.retain(|(key, _)| key != name.as_str());
        env.push((name.into(), value.into()));
    }

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
    let messages = state.join("full-messages.json");
    host.stream(
        &Invocation::new(
            "cargo",
            &[
                "test",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
                "--no-run",
                "--message-format=json-render-diagnostics",
            ],
            &root,
        )
        .with_env(&env),
        Some(&messages),
    )
    .await?;
    let inventory = state.join("inventory.json");
    write_inventory(&metadata, &messages, &target, &inventory)
        .context("full coverage inventory validation failed")?;
    Ok(Prepared {
        root,
        target,
        state,
        llvm_cov,
        env,
        inventory,
    })
}

async fn run_shard<H: Host>(
    host: &mut H,
    root: &Path,
    helper: &Path,
    common: &Common,
    inputs: &ShardInputs,
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
    let known = workspace_packages();
    for package in &inputs.packages {
        ensure!(
            known.contains(package.as_str()),
            "unknown coverage package {package}"
        );
    }
    let expected = shard_packages(&inputs.shard)?;
    ensure!(
        inputs.packages == expected,
        "coverage shard {} packages {:?} differ from its SHARDS entry {:?}",
        inputs.shard,
        inputs.packages,
        expected
    );

    let prepared = prepare(host, root, common, state_slot).await?;
    let Prepared {
        root,
        target,
        state,
        llvm_cov,
        env,
        inventory,
    } = &prepared;
    let selection = state.join("selection.json");
    write_selection(inventory, &inputs.packages, &selection)
        .context("selected coverage inventory validation failed")?;

    let rustc = host
        .capture(&Invocation::new("rustc", &["-vV"], root))
        .await?;
    let hosts: Vec<_> = rustc
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect();
    let [host_target] = hosts[..] else {
        bail!("Rust identity did not contain one host target");
    };
    let ledger = state.join("runner-ledger.jsonl");
    let runner_config = state.join("runner-config.toml");
    // The runner's test deadline sits inside the hosted job limit, so a
    // stalled test fails here with evidence instead of a host cancellation.
    write_runner_config(&RunnerConfigOptions {
        root,
        host: host_target,
        helper,
        inventory,
        selection: &selection,
        target_dir: target,
        ledger: &ledger,
        diagnostics,
        job_started: inputs.job_started,
        job_minutes: inputs.job_minutes,
        output: &runner_config,
    })
    .context("Cargo runner configuration failed")?;
    // Compile-phase profiles are not test evidence. The run below executes the
    // same full Cargo graph while the runner omits unassigned test binaries.
    let discarded = discard_compile_profiles(target).context("compile-profile isolation failed")?;
    eprintln!(
        "coverage shard {}: discarded {discarded} compile-phase profiles",
        inputs.shard
    );
    let config = runner_config
        .to_str()
        .context("Cargo runner configuration path is not UTF-8")?;
    host.stream(
        &Invocation::new(
            "cargo",
            &[
                "--config",
                config,
                "test",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
                "--no-fail-fast",
            ],
            root,
        )
        .with_env(env),
        None,
    )
    .await
    .context("coverage shard tests failed")?;
    validate_run_ledger(inventory, &selection, &ledger)
        .context("Cargo runner ledger validation failed")?;
    // The receipt writes attempt-<n> inside the artifact root, so the uploaded
    // artifact names its own attempt in either download layout.
    host.receipt(&ReceiptOptions {
        root,
        inventory,
        selection: &selection,
        ledger: &ledger,
        profiles: target,
        shard: &inputs.shard,
        run_attempt: &common.attempt,
        expected_source: &common.source,
        llvm_cov,
        output: &inputs.output,
    })
    .await
    .context("coverage shard receipt failed")
}

async fn run_collect<H: Host>(
    host: &mut H,
    root: &Path,
    common: &Common,
    inputs: &CollectInputs,
) -> Result<()> {
    artifact_os_label(&common.os)?;
    let mut state = None;
    let prepared = prepare(host, root, common, &mut state).await?;
    discard_compile_profiles(&prepared.target).context("compile-profile isolation failed")?;
    // Each shard contributes its latest uploaded attempt no later than this one.
    let selected = host
        .collect(&CollectOptions {
            root: &prepared.root,
            inventory: &prepared.inventory,
            inputs: &inputs.inputs,
            target_dir: &prepared.target,
            expected_source: &common.source,
            max_attempt: &common.attempt,
            artifact_os: &common.os,
            llvm_cov: &prepared.llvm_cov,
        })
        .await
        .context("coverage shard aggregation failed")?;
    for (shard, attempt) in selected {
        println!("coverage shard {shard}: accepted run attempt {attempt}");
    }
    match fs::symlink_metadata(&inputs.report) {
        Ok(_) => bail!(
            "coverage report already exists: {}",
            inputs.report.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect the coverage report path"),
    }
    if let Some(parent) = inputs.report.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let report = inputs
        .report
        .to_str()
        .context("coverage report path is not UTF-8")?;
    host.stream(
        &Invocation::new(
            &prepared.llvm_cov,
            &[
                "llvm-cov",
                "report",
                "--failure-mode",
                "any",
                "--fail-under-lines",
                "90",
                "--lcov",
                "--output-path",
                report,
            ],
            &prepared.root,
        )
        .with_env(&prepared.env),
        None,
    )
    .await
    .context("workspace coverage report failed")?;
    ensure!(
        fs::symlink_metadata(&inputs.report).is_ok_and(|metadata| metadata.file_type().is_file()),
        "workspace coverage report was not written to {}",
        inputs.report.display()
    );
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
    use super::super::{
        Inventory, RunnerRecord, SCHEMA, Selection, append_runner_record, read_json, unix_now,
    };
    use super::*;
    use tempfile::TempDir;

    const MACOS_CAPTURED: &str =
        include_str!("../../tests/fixtures/coverage-show-env/macos-captured.pwsh.txt");
    const LINUX_AUTHORED: &str =
        include_str!("../../tests/fixtures/coverage-show-env/linux-authored.pwsh.txt");
    const WINDOWS_AUTHORED: &str =
        include_str!("../../tests/fixtures/coverage-show-env/windows-authored.pwsh.txt");

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
        let names: Vec<_> = parse_show_env(MACOS_CAPTURED)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
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

        // Authored from the 0.9.1 `--pwsh` writer (cli.rs `ShowEnvFormat::Pwsh`
        // and main.rs `set_env`) with GitHub-hosted runner paths; not captured.
        let linux = variables(LINUX_AUTHORED);
        assert_eq!(
            linux["CARGO_LLVM_COV_TARGET_DIR"],
            "/home/runner/work/_temp/kuru-coverage-memory-1"
        );
        assert_eq!(linux.len(), macos.len());
        let windows = variables(WINDOWS_AUTHORED);
        assert_eq!(
            windows["CARGO_LLVM_COV_TARGET_DIR"],
            r"D:\a\_temp\kuru-coverage-memory-1"
        );
        assert!(windows["RUSTC_WRAPPER"].ends_with(r"\bin\cargo-llvm-cov.exe"));
        assert_eq!(
            windows["LLVM_PROFILE_FILE"],
            r"D:\a\_temp\kuru-coverage-memory-1\kuru-%p-%4m.profraw"
        );
        assert_eq!(windows.len(), macos.len());
        // The trimmed capture parses identically.
        assert_eq!(variables(MACOS_CAPTURED.trim_end()), macos);
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

    const SHARD: &str = "connectors-core-platform";
    const PACKAGES: [&str; 4] = [
        "kuru-connectors",
        "kuru-core",
        "kuru-memory",
        "kuru-platform",
    ];
    const HOST_TRIPLE: &str = "x86_64-unknown-linux-gnu";

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Step {
        BinPaths,
        Version,
        VerifySource,
        ShowEnv,
        Metadata,
        NoRun,
        Rustc,
        TestRun,
        Receipt,
        Collect,
        Report,
    }

    /// Records every requested process interaction and simulates its effect.
    struct Fake {
        workspace: PathBuf,
        llvm_cov_dir: PathBuf,
        steps: Vec<Step>,
        invocations: Vec<Invocation>,
        fail: Option<Step>,
        version: String,
        show_env: Option<String>,
        bin_paths: Option<String>,
        rustc: String,
        receipts: Vec<(String, String, PathBuf, PathBuf)>,
        collected: Vec<(String, String, PathBuf)>,
        write_report: bool,
    }

    impl Fake {
        fn new(temp: &Path) -> Self {
            let workspace = temp.join("workspace");
            for package in PACKAGES {
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
                steps: Vec::new(),
                invocations: Vec::new(),
                fail: None,
                version: LLVM_COV_VERSION.to_owned(),
                show_env: None,
                bin_paths: None,
                rustc: format!("rustc 1.98.1\nhost: {HOST_TRIPLE}"),
                receipts: Vec::new(),
                collected: Vec::new(),
                write_report: true,
            }
        }

        fn step(&mut self, step: Step) -> Result<()> {
            self.steps.push(step);
            ensure!(self.fail != Some(step), "injected {step:?} failure");
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
            let packages: Vec<_> = PACKAGES
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
                "workspace_members": PACKAGES.iter().map(|package| format!("{package}-id")).collect::<Vec<_>>(),
                "workspace_root": self.workspace,
            })
        }

        fn messages(&self, target: &Path) -> Vec<u8> {
            let mut bytes = Vec::new();
            for package in PACKAGES {
                let name = Self::crate_name(package);
                let executable = target.join("debug/deps").join(format!("{name}-0a1b"));
                serde_json::to_writer(
                    &mut bytes,
                    &serde_json::json!({
                        "reason": "compiler-artifact",
                        "package_id": format!("{package}-id"),
                        "target": {
                            "kind": ["lib"], "crate_types": ["lib"], "name": name,
                            "src_path": self.workspace.join("packages").join(package).join("src/lib.rs"),
                            "edition": "2024", "doc": true, "doctest": true, "test": true
                        },
                        "profile": {
                            "opt_level": "0", "debuginfo": 2, "debug_assertions": true,
                            "overflow_checks": true, "test": true
                        },
                        "features": [],
                        "filenames": [executable],
                        "executable": executable,
                        "fresh": false
                    }),
                )
                .unwrap();
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(b"{\"reason\":\"build-finished\",\"success\":true}\n");
            bytes
        }

        /// Act as Cargo driving the task-private runner over the inventory.
        fn run_tests(&self, invocation: &Invocation) -> Result<()> {
            let config = PathBuf::from(&invocation.args[1]);
            let state = config.parent().unwrap();
            let runner: toml::Value = toml::from_str(&fs::read_to_string(&config)?)?;
            let command = runner["target"][HOST_TRIPLE]["runner"].as_array().unwrap();
            assert_eq!(command[1].as_str(), Some("coverage"));
            assert_eq!(command[2].as_str(), Some("dispatch"));
            let inventory: Inventory = read_json(&state.join("inventory.json"))?;
            let selection: Selection = read_json(&state.join("selection.json"))?;
            for artifact in &inventory.artifacts {
                let selected = selection.packages.contains(&artifact.package);
                append_runner_record(
                    &state.join("runner-ledger.jsonl"),
                    &RunnerRecord {
                        schema: SCHEMA,
                        executable: artifact.executable.clone().unwrap(),
                        action: if selected { "run" } else { "omit" }.to_owned(),
                        cwd: artifact.package_root.clone(),
                        args: Vec::new(),
                        success: true,
                        status_code: Some(0),
                    },
                )?;
            }
            fs::write(
                Self::target(invocation).join("kuru-1-1.profraw"),
                b"profile",
            )?;
            Ok(())
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
                self.step(Step::BinPaths)?;
                return Ok(self
                    .bin_paths
                    .clone()
                    .unwrap_or_else(|| self.llvm_cov_dir.display().to_string()));
            }
            if invocation.program == "rustc" {
                self.step(Step::Rustc)?;
                return Ok(self.rustc.clone());
            }
            match args[..] {
                ["llvm-cov", "--version"] => {
                    self.step(Step::Version)?;
                    Ok(self.version.clone())
                }
                ["llvm-cov", "show-env", "--pwsh"] => {
                    self.step(Step::ShowEnv)?;
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
                    self.step(Step::Metadata)?;
                    fs::write(stdout.unwrap(), serde_json::to_vec(&self.metadata())?)?;
                }
                ["test", .., "--no-run", _] => {
                    self.step(Step::NoRun)?;
                    // A compile-phase profile that must be discarded.
                    fs::write(
                        Self::target(invocation).join("kuru-0-0.profraw"),
                        b"compile",
                    )?;
                    fs::write(stdout.unwrap(), self.messages(&Self::target(invocation)))?;
                }
                ["--config", _, "test", ..] => {
                    assert!(stdout.is_none());
                    self.step(Step::TestRun)?;
                    self.run_tests(invocation)?;
                }
                ["llvm-cov", "report", .., output] => {
                    assert!(stdout.is_none());
                    self.step(Step::Report)?;
                    if self.write_report {
                        fs::write(output, b"TN:\n")?;
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
            llvm_cov: &Path,
        ) -> Result<()> {
            assert_eq!(root, self.workspace);
            assert_eq!(source, "abc123");
            assert!(llvm_cov.starts_with(&self.llvm_cov_dir));
            self.step(Step::VerifySource)
        }

        async fn receipt(&mut self, options: &ReceiptOptions<'_>) -> Result<()> {
            self.step(Step::Receipt)?;
            // The runner ledger and profiles are exactly what write_receipt reads.
            validate_run_ledger(options.inventory, options.selection, options.ledger)?;
            assert!(options.profiles.join("kuru-1-1.profraw").is_file());
            assert!(!options.profiles.join("kuru-0-0.profraw").exists());
            self.receipts.push((
                options.shard.to_owned(),
                options.run_attempt.to_owned(),
                options.output.to_owned(),
                options.llvm_cov.to_owned(),
            ));
            Ok(())
        }

        async fn collect(&mut self, options: &CollectOptions<'_>) -> Result<Vec<(String, u64)>> {
            self.step(Step::Collect)?;
            assert!(!options.target_dir.join("kuru-0-0.profraw").exists());
            self.collected.push((
                options.artifact_os.to_owned(),
                options.max_attempt.to_owned(),
                options.inputs.to_owned(),
            ));
            Ok(vec![(SHARD.to_owned(), 2)])
        }
    }

    struct Scenario {
        temp: TempDir,
        fake: Fake,
        helper: PathBuf,
        vars: BTreeMap<String, String>,
    }

    impl Scenario {
        fn new(mode: Mode) -> Self {
            let temp = TempDir::new().unwrap();
            let fake = Fake::new(temp.path());
            let helper = temp.path().join("kuru-delivery-helper");
            fs::write(&helper, b"").unwrap();
            let jobs = temp.path().join("job");
            let mut vars = BTreeMap::new();
            let mut set = |name: &str, value: String| {
                vars.insert(format!("{INPUT_PREFIX}{name}"), value);
            };
            set("TARGET", jobs.join("target").display().to_string());
            set("SOURCE", "abc123".to_owned());
            set("ATTEMPT", "2".to_owned());
            set("OS", "ubuntu-latest".to_owned());
            match mode {
                Mode::Shard => {
                    set(
                        "PACKAGES",
                        "kuru-platform,kuru-core, kuru-connectors,kuru-core".to_owned(),
                    );
                    set("SHARD", SHARD.to_owned());
                    set("OUTPUT", jobs.join("evidence").display().to_string());
                    set(
                        "DIAGNOSTICS",
                        jobs.join("diagnostics").display().to_string(),
                    );
                    set("JOB_STARTED", (unix_now().unwrap() - 60).to_string());
                    set("JOB_MINUTES", "75".to_owned());
                }
                Mode::Collect => {
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
            Self {
                temp,
                fake,
                helper,
                vars,
            }
        }

        /// A job path joined per component, as `std::path::absolute` spells it.
        fn job(&self, name: &str) -> PathBuf {
            name.split('/')
                .fold(self.temp.path().join("job"), |path, part| path.join(part))
        }

        async fn run(&mut self, mode: Mode) -> Result<()> {
            let vars: Vec<(OsString, OsString)> = self
                .vars
                .iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect();
            let root = self.fake.workspace.clone();
            run(mode, &root, &self.helper, vars, &mut self.fake).await
        }

        fn diagnostics(&self) -> BTreeSet<String> {
            fs::read_dir(self.job("diagnostics"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect()
        }

        fn failure(&self) -> String {
            fs::read_to_string(self.job("diagnostics").join("failure.txt")).unwrap()
        }
    }

    #[tokio::test]
    async fn shard_sequences_the_full_inventory_runner_and_receipt() {
        let mut scenario = Scenario::new(Mode::Shard);
        scenario.run(Mode::Shard).await.unwrap();
        let fake = &scenario.fake;
        assert_eq!(
            fake.steps,
            [
                Step::BinPaths,
                Step::Version,
                Step::VerifySource,
                Step::ShowEnv,
                Step::Metadata,
                Step::NoRun,
                Step::Rustc,
                Step::TestRun,
                Step::Receipt,
            ]
        );
        let target = resolved_directory(&scenario.job("target")).unwrap();
        let state = target.join(STATE);
        for name in ["metadata.json", "full-messages.json", "inventory.json"] {
            assert!(state.join(name).is_file(), "{name}");
        }
        let selection: Selection = read_json(&state.join("selection.json")).unwrap();
        assert_eq!(
            selection.packages,
            ["kuru-connectors", "kuru-core", "kuru-platform"]
        );
        let config: toml::Value =
            toml::from_str(&fs::read_to_string(state.join("runner-config.toml")).unwrap()).unwrap();
        let runner: Vec<_> = config["target"][HOST_TRIPLE]["runner"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert_eq!(runner[0], scenario.helper.to_str().unwrap());
        let diagnostics = resolved_directory(&scenario.job("diagnostics")).unwrap();
        let flag = runner
            .iter()
            .position(|arg| *arg == "--diagnostics")
            .unwrap();
        assert_eq!(runner[flag + 1], diagnostics.to_str().unwrap());
        let flag = runner
            .iter()
            .position(|arg| *arg == "--target-dir")
            .unwrap();
        assert_eq!(runner[flag + 1], target.to_str().unwrap());

        // show-env ran with the target selected; every later Cargo child gets
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
        for invocation in cargo {
            assert_eq!(invocation.cwd, fake.workspace);
            assert_eq!(
                Fake::env(invocation, "LLVM_PROFILE_FILE").unwrap(),
                target.join("kuru-%p-%4m.profraw").as_os_str()
            );
            assert_eq!(
                Fake::env(invocation, "CARGO_LLVM_COV_TARGET_DIR").unwrap(),
                target.as_os_str()
            );
            assert_eq!(
                Fake::env(invocation, "RUSTC_WRAPPER").unwrap(),
                "/fake/cargo-llvm-cov"
            );
            let names: BTreeSet<_> = invocation.env.iter().map(|(key, _)| key).collect();
            assert_eq!(names.len(), invocation.env.len(), "duplicate child env");
        }
        let run = fake.invocations.last().unwrap();
        assert_eq!(
            run.args,
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
        // Captures that are not Cargo children never see the coverage env.
        for invocation in fake
            .invocations
            .iter()
            .filter(|invocation| invocation.program == "mise" || invocation.program == "rustc")
        {
            assert!(invocation.env.is_empty(), "{}", invocation.describe());
        }
        assert_eq!(
            fake.receipts,
            [(
                SHARD.to_owned(),
                "2".to_owned(),
                scenario.job("evidence"),
                fake.llvm_cov_dir
                    .join(format!("cargo-llvm-cov{}", std::env::consts::EXE_SUFFIX)),
            )]
        );
        assert!(scenario.diagnostics().is_empty());
        assert!(!fake.invocations.iter().any(|invocation| {
            invocation
                .args
                .iter()
                .any(|arg| arg == "--fail-under-lines")
        }));
    }

    #[tokio::test]
    async fn collect_sequences_one_report_for_this_os() {
        let mut scenario = Scenario::new(Mode::Collect);
        scenario.run(Mode::Collect).await.unwrap();
        let fake = &scenario.fake;
        assert_eq!(
            fake.steps,
            [
                Step::BinPaths,
                Step::Version,
                Step::VerifySource,
                Step::ShowEnv,
                Step::Metadata,
                Step::NoRun,
                Step::Collect,
                Step::Report,
            ]
        );
        assert_eq!(
            fake.collected,
            [(
                "ubuntu-latest".to_owned(),
                "2".to_owned(),
                scenario.job("inputs")
            )]
        );
        let gates: Vec<_> = fake
            .invocations
            .iter()
            .filter(|invocation| {
                invocation
                    .args
                    .iter()
                    .any(|arg| arg == "--fail-under-lines")
            })
            .collect();
        assert_eq!(gates.len(), 1, "the 90% gate is enforced exactly once");
        let report = scenario.job("report/coverage.lcov");
        assert_eq!(
            gates[0].args,
            [
                "llvm-cov",
                "report",
                "--failure-mode",
                "any",
                "--fail-under-lines",
                "90",
                "--lcov",
                "--output-path",
                report.to_str().unwrap(),
            ]
            .map(OsString::from)
        );
        assert!(Fake::env(gates[0], "CARGO_LLVM_COV_TARGET_DIR").is_some());
        assert_eq!(fs::read(&report).unwrap(), b"TN:\n");
        assert!(!scenario.job("diagnostics").exists());
    }

    #[tokio::test]
    async fn inputs_are_required_per_mode_before_any_effect() {
        let mut shard = Scenario::new(Mode::Shard);
        for name in ["SHARD", "JOB_MINUTES", "OS"] {
            shard.vars.remove(&format!("{INPUT_PREFIX}{name}"));
        }
        shard
            .vars
            .insert(format!("{INPUT_PREFIX}PACKAGES"), "  ".to_owned());
        let error = shard.run(Mode::Shard).await.unwrap_err().to_string();
        assert_eq!(
            error,
            "coverage shard requires KURU_COVERAGE_OS, KURU_COVERAGE_SHARD, \
             KURU_COVERAGE_PACKAGES, KURU_COVERAGE_JOB_MINUTES"
        );
        assert!(shard.fake.steps.is_empty());
        assert!(!shard.job("diagnostics").exists());

        let mut collect = Scenario::new(Mode::Collect);
        collect.vars.remove(&format!("{INPUT_PREFIX}REPORT"));
        let error = collect.run(Mode::Collect).await.unwrap_err().to_string();
        assert_eq!(error, "coverage collect requires KURU_COVERAGE_REPORT");

        let mut minutes = Scenario::new(Mode::Shard);
        minutes
            .vars
            .insert(format!("{INPUT_PREFIX}JOB_MINUTES"), "ninety".to_owned());
        let error = minutes.run(Mode::Shard).await.unwrap_err().to_string();
        assert!(error.contains("JOB_MINUTES must be"), "{error}");
        assert!(!minutes.job("diagnostics").exists());

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let error = Inputs::parse(
                Mode::Collect,
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
    async fn existing_directories_and_reports_are_refused() {
        // Diagnostics that already exist are neither reused nor written.
        let mut diagnostics = Scenario::new(Mode::Shard);
        fs::create_dir_all(diagnostics.job("diagnostics")).unwrap();
        let error = diagnostics.run(Mode::Shard).await.unwrap_err().to_string();
        assert!(
            error.contains("coverage diagnostics already exists"),
            "{error}"
        );
        assert!(diagnostics.diagnostics().is_empty());
        assert!(diagnostics.fake.steps.is_empty());

        // An existing target is refused inside the diagnostics guard.
        let mut target = Scenario::new(Mode::Shard);
        fs::create_dir_all(target.job("target")).unwrap();
        let error = target.run(Mode::Shard).await.unwrap_err().to_string();
        assert!(error.contains("coverage target already exists"), "{error}");
        assert_eq!(
            target.diagnostics(),
            BTreeSet::from(["failure.txt".to_owned()])
        );
        assert!(target.failure().contains("coverage target already exists"));
        assert!(target.fake.steps.is_empty());

        let mut collect_target = Scenario::new(Mode::Collect);
        fs::create_dir_all(collect_target.job("target")).unwrap();
        let error = collect_target
            .run(Mode::Collect)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("coverage target already exists"), "{error}");

        // An existing report is refused before the report command runs.
        let mut report = Scenario::new(Mode::Collect);
        fs::create_dir_all(report.job("report")).unwrap();
        fs::write(report.job("report/coverage.lcov"), b"old").unwrap();
        let error = report.run(Mode::Collect).await.unwrap_err().to_string();
        assert!(error.contains("coverage report already exists"), "{error}");
        assert_eq!(report.fake.steps.last(), Some(&Step::Collect));
        assert_eq!(
            fs::read(report.job("report/coverage.lcov")).unwrap(),
            b"old"
        );

        // A report command that succeeds without writing the report fails.
        let mut missing = Scenario::new(Mode::Collect);
        missing.fake.write_report = false;
        let error = missing.run(Mode::Collect).await.unwrap_err().to_string();
        assert!(error.contains("was not written"), "{error}");
    }

    #[tokio::test]
    async fn shard_inputs_must_name_their_exact_shards_entry() {
        for (shard, packages, reason) in [
            (
                SHARD,
                "kuru-core,kuru-nope",
                "unknown coverage package kuru-nope",
            ),
            (
                SHARD,
                "kuru-core,kuru-platform",
                "differ from its SHARDS entry",
            ),
            ("elsewhere", "kuru-core", "unknown coverage shard elsewhere"),
        ] {
            let mut scenario = Scenario::new(Mode::Shard);
            scenario
                .vars
                .insert(format!("{INPUT_PREFIX}SHARD"), shard.to_owned());
            scenario
                .vars
                .insert(format!("{INPUT_PREFIX}PACKAGES"), packages.to_owned());
            let error = scenario.run(Mode::Shard).await.unwrap_err().to_string();
            assert!(error.contains(reason), "{packages}: {error}");
            assert!(scenario.failure().contains(reason));
            assert!(scenario.fake.steps.is_empty());
            assert!(!scenario.job("target").exists());
        }
        for attempt in ["0", "02", "x"] {
            let mut scenario = Scenario::new(Mode::Shard);
            scenario
                .vars
                .insert(format!("{INPUT_PREFIX}ATTEMPT"), attempt.to_owned());
            let error = scenario.run(Mode::Shard).await.unwrap_err().to_string();
            assert!(error.contains("run attempt"), "{attempt}: {error}");
        }
        let mut helper = Scenario::new(Mode::Shard);
        helper.helper = helper.temp.path().join("missing-helper");
        let error = helper.run(Mode::Shard).await.unwrap_err().to_string();
        assert!(error.contains("coverage helper"), "{error}");

        let mut os = Scenario::new(Mode::Collect);
        os.vars
            .insert(format!("{INPUT_PREFIX}OS"), "Ubuntu".to_owned());
        let error = os.run(Mode::Collect).await.unwrap_err().to_string();
        assert!(error.contains("[a-z0-9-]+"), "{error}");
        assert!(os.fake.steps.is_empty());
    }

    #[tokio::test]
    async fn every_shard_failure_leaves_its_diagnostics() {
        let early = BTreeSet::from(["failure.txt".to_owned()]);
        let manifests = BTreeSet::from(
            [
                "failure.txt",
                "inventory.json",
                "selection.json",
                "runner-config.toml",
            ]
            .map(str::to_owned),
        );
        let mut ledger = manifests.clone();
        ledger.insert("runner-ledger.jsonl".to_owned());
        for (step, expected) in [
            (Step::BinPaths, &early),
            (Step::Version, &early),
            (Step::VerifySource, &early),
            (Step::ShowEnv, &early),
            (Step::Metadata, &early),
            (Step::NoRun, &early),
            (Step::TestRun, &manifests),
            (Step::Receipt, &ledger),
        ] {
            let mut scenario = Scenario::new(Mode::Shard);
            scenario.fake.fail = Some(step);
            let error = format!("{:#}", scenario.run(Mode::Shard).await.unwrap_err());
            let injected = format!("injected {step:?} failure");
            assert!(error.contains(&injected), "{step:?}: {error}");
            assert_eq!(&scenario.diagnostics(), expected, "{step:?}");
            assert!(scenario.failure().contains(&injected), "{step:?}");
            assert_eq!(scenario.fake.steps.last(), Some(&step));
        }
        let mut rustc = Scenario::new(Mode::Shard);
        rustc.fake.rustc = "rustc 1.98.1".to_owned();
        let error = rustc.run(Mode::Shard).await.unwrap_err().to_string();
        assert!(error.contains("one host target"), "{error}");
        assert!(rustc.diagnostics().contains("selection.json"));
    }

    #[tokio::test]
    async fn toolchain_and_coverage_environment_are_checked_before_cargo() {
        type Setup = fn(&mut Fake);
        let cases: [(Setup, &str); 6] = [
            (
                |fake| fake.version = "cargo-llvm-cov 0.9.0".to_owned(),
                "expected cargo-llvm-cov 0.9.1, got cargo-llvm-cov 0.9.0",
            ),
            (
                |fake| fake.bin_paths = Some(String::new()),
                "did not name one cargo:cargo-llvm-cov@0.9.1 directory",
            ),
            (
                |fake| fake.bin_paths = Some("/a\n/b".to_owned()),
                "did not name one cargo:cargo-llvm-cov@0.9.1 directory",
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
        ];
        for (setup, reason) in cases {
            let mut scenario = Scenario::new(Mode::Shard);
            setup(&mut scenario.fake);
            let error = format!("{:#}", scenario.run(Mode::Shard).await.unwrap_err());
            assert!(error.contains(reason), "{reason}: {error}");
            assert!(
                !scenario.fake.steps.contains(&Step::Metadata),
                "{reason}: Cargo ran"
            );
            assert_eq!(
                scenario.diagnostics(),
                BTreeSet::from(["failure.txt".to_owned()])
            );
        }
        let mut omitted = Scenario::new(Mode::Collect);
        omitted.fake.show_env = Some(pwsh("CARGO_LLVM_COV", "1"));
        let error = omitted.run(Mode::Collect).await.unwrap_err().to_string();
        assert!(error.contains("omits CARGO_LLVM_COV_TARGET_DIR"), "{error}");
    }

    #[tokio::test]
    async fn collect_failures_propagate_without_a_report() {
        for step in [Step::Collect, Step::Report, Step::NoRun] {
            let mut scenario = Scenario::new(Mode::Collect);
            scenario.fake.fail = Some(step);
            let error = format!("{:#}", scenario.run(Mode::Collect).await.unwrap_err());
            assert!(
                error.contains(&format!("injected {step:?} failure")),
                "{error}"
            );
            assert!(!scenario.job("report/coverage.lcov").exists());
        }
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
        fs::create_dir(state.join("selection.json")).unwrap();
        let error = anyhow::anyhow!("inner").context("outer");
        record_failure(&diagnostics, Some(&state), &error);
        let failure = fs::read_to_string(diagnostics.join("failure.txt")).unwrap();
        assert!(
            failure.contains("outer") && failure.contains("inner"),
            "{failure}"
        );
        assert!(diagnostics.join("inventory.json").is_file());
        assert!(!diagnostics.join("selection.json").exists());

        // Unwritable diagnostics are reported to stderr without panicking.
        let locked = temp.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        record_failure(&locked, Some(&state), &error);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    }
}
