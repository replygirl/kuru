//! Fail-closed manifests for sharded native coverage.

pub mod orchestrate;

use crate::{archive, command};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    process::ExitStatus,
    time::Duration,
};

const SCHEMA: u32 = 1;
const LLVM_COV_VERSION: &str = "cargo-llvm-cov 0.9.1";
const INSTRUMENTATION: &str = "cargo-test-no-run;workspace;all-targets;all-features;locked;instrument-coverage;exact-libtest-executables";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const COMMAND_OUTPUT_LIMIT: usize = 64 * 1024;
const JSON_LIMIT: u64 = 16 * 1024 * 1024;
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const PROFILE_LIMIT: u64 = 512 * 1024 * 1024;
const PROFILE_TOTAL_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
const PROFILE_COUNT_LIMIT: usize = 4096;
const RUNNER_LEDGER_LIMIT: u64 = 16 * 1024 * 1024;
const RUNNER_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
/// Time kept between the shard's test deadline and its hosted job limit. It covers
/// tree termination and cleanup, fast refusal of the remaining Cargo test
/// executables, evidence copying and the diagnostics upload.
const EVIDENCE_RESERVE: Duration = Duration::from_secs(10 * 60);
const TEST_LOG_LIMIT: u64 = 32 * 1024 * 1024;
const RECENT_RESULT_LIMIT: usize = 20;
const PENDING_LINE_LIMIT: usize = 64 * 1024;

pub const SHARDS: [(&str, &[&str]); 5] = [
    ("delivery-archive", &["kuru-delivery", "kuru-archive"]),
    ("application", &["kuru"]),
    ("memory", &["kuru-memory"]),
    ("runtime", &["kuru-runtime"]),
    (
        "connectors-core-platform",
        &["kuru-connectors", "kuru-core", "kuru-platform"],
    ),
];

/// Every workspace package, derived from [`SHARDS`] so the two cannot drift.
pub fn workspace_packages() -> BTreeSet<&'static str> {
    SHARDS
        .iter()
        .flat_map(|(_, packages)| packages.iter().copied())
        .collect()
}

/// Validate the hosted OS label that names this OS's coverage artifacts.
pub fn artifact_os_label(label: &str) -> Result<&str> {
    ensure!(
        !label.is_empty()
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
        "coverage artifact OS label {label:?} must match [a-z0-9-]+"
    );
    Ok(label)
}

#[derive(Clone, Debug, Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
    workspace_members: Vec<String>,
    workspace_root: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
struct MetadataPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
    targets: Vec<MetadataTarget>,
}

#[derive(Clone, Debug, Deserialize)]
struct MetadataTarget {
    kind: Vec<String>,
    name: String,
    test: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct CargoMessage {
    reason: String,
    package_id: Option<String>,
    target: Option<CargoTarget>,
    profile: Option<CargoProfile>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    filenames: Vec<PathBuf>,
    executable: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
struct CargoTarget {
    kind: Vec<String>,
    crate_types: Vec<String>,
    name: String,
    src_path: PathBuf,
    edition: String,
    doc: bool,
    doctest: bool,
    test: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct CargoProfile {
    opt_level: String,
    debuginfo: serde_json::Value,
    debug_assertions: bool,
    overflow_checks: bool,
    test: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Artifact {
    package: String,
    package_root: String,
    target_name: String,
    target_kind: Vec<String>,
    crate_types: Vec<String>,
    source: String,
    edition: String,
    doc: bool,
    doctest: bool,
    target_test: bool,
    profile_test: bool,
    opt_level: String,
    debuginfo: String,
    debug_assertions: bool,
    overflow_checks: bool,
    features: Vec<String>,
    filenames: Vec<String>,
    executable: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Inventory {
    schema: u32,
    workspace_packages: Vec<String>,
    artifacts: Vec<Artifact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Selection {
    schema: u32,
    inventory_sha256: String,
    packages: Vec<String>,
    artifacts: Vec<Artifact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct RunnerRecord {
    schema: u32,
    executable: String,
    action: String,
    cwd: String,
    args: Vec<String>,
    success: bool,
    status_code: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProfileReceipt {
    name: String,
    bytes: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Receipt {
    schema: u32,
    shard: String,
    run_attempt: String,
    packages: Vec<String>,
    source: String,
    tree: String,
    cargo_lock_sha256: String,
    rustc: String,
    cargo: String,
    target: String,
    cargo_llvm_cov: String,
    instrumentation: String,
    target_os: String,
    inventory_sha256: String,
    selection_sha256: String,
    runner_ledger_sha256: String,
    profiles: Vec<ProfileReceipt>,
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file(),
        "{} is not a regular file",
        path.display()
    );
    ensure!(
        metadata.len() <= limit,
        "{} exceeds {limit} bytes",
        path.display()
    );
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    ensure!(
        bytes.len() as u64 <= limit,
        "{} grew beyond {limit} bytes",
        path.display()
    );
    Ok(bytes)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    serde_json::from_slice(&read_bounded(path, JSON_LIMIT)?)
        .with_context(|| format!("parse {}", path.display()))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn digest_json(value: &impl Serialize) -> Result<String> {
    Ok(archive::digest(&serde_json::to_vec(value)?))
}

fn normalized_relative(root: &Path, path: &Path, label: &str) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{label} {} is outside {}", path.display(), root.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .with_context(|| format!("{label} contains non-UTF-8 text"))?,
            ),
            _ => bail!("{label} contains a non-normal path component"),
        }
    }
    ensure!(!parts.is_empty(), "{label} cannot name its root");
    Ok(parts.join("/"))
}

fn metadata(path: &Path) -> Result<Metadata> {
    let metadata: Metadata = read_json(path)?;
    let members: BTreeSet<_> = metadata.workspace_members.iter().collect();
    let mut names = BTreeSet::new();
    for package in &metadata.packages {
        if members.contains(&package.id) {
            ensure!(
                names.insert(package.name.as_str()),
                "workspace package name {} is duplicated",
                package.name
            );
            let manifest_bytes = read_bounded(&package.manifest_path, MANIFEST_LIMIT)?;
            let manifest: toml::Value = toml::from_str(std::str::from_utf8(&manifest_bytes)?)?;
            for section in ["lib", "bin", "test", "example", "bench"] {
                let Some(value) = manifest.get(section) else {
                    continue;
                };
                let tables: Vec<_> = match value {
                    toml::Value::Table(table) => vec![table],
                    toml::Value::Array(values) => {
                        values.iter().filter_map(toml::Value::as_table).collect()
                    }
                    _ => bail!("{} has an invalid [{section}] section", package.name),
                };
                ensure!(
                    tables
                        .iter()
                        .all(|table| table.get("harness").and_then(toml::Value::as_bool)
                            != Some(false)),
                    "{} declares unsupported harness=false test execution",
                    package.name
                );
            }
            for target in package.targets.iter().filter(|target| target.test) {
                ensure!(
                    target.kind.len() == 1
                        && matches!(target.kind[0].as_str(), "lib" | "bin" | "test"),
                    "{} target {} has unsupported Cargo test semantics: {:?}",
                    package.name,
                    target.name,
                    target.kind
                );
            }
        }
    }
    ensure!(
        names.len() == members.len(),
        "workspace metadata omits a member package"
    );
    Ok(metadata)
}

fn artifact_inventory(
    metadata: &Metadata,
    messages: &Path,
    target_dir: &Path,
) -> Result<Inventory> {
    let workspace: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| (package.id.as_str(), package))
        .collect();
    let mut artifacts = BTreeSet::new();
    let reader = BufReader::new(File::open(messages)?);
    for (index, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("read Cargo message line {}", index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let message: CargoMessage = serde_json::from_str(&line)
            .with_context(|| format!("parse Cargo message line {}", index + 1))?;
        if message.reason != "compiler-artifact" {
            continue;
        }
        let Some(package_id) = message.package_id.as_deref() else {
            bail!("compiler artifact omits package identity");
        };
        let Some(package) = workspace.get(package_id).copied() else {
            continue;
        };
        let target = message.target.context("compiler artifact omits target")?;
        let profile = message.profile.context("compiler artifact omits profile")?;
        let mut features = message.features;
        features.sort();
        features.dedup();
        let mut filenames = message
            .filenames
            .iter()
            .map(|path| normalized_relative(target_dir, path, "artifact filename"))
            .collect::<Result<Vec<_>>>()?;
        filenames.sort();
        filenames.dedup();
        ensure!(
            !filenames.is_empty(),
            "workspace compiler artifact has no filenames"
        );
        let executable = message
            .executable
            .as_deref()
            .map(|path| normalized_relative(target_dir, path, "artifact executable"))
            .transpose()?;
        artifacts.insert(Artifact {
            package: package.name.clone(),
            package_root: normalized_relative(
                &metadata.workspace_root,
                package
                    .manifest_path
                    .parent()
                    .context("workspace manifest has no parent")?,
                "package root",
            )?,
            target_name: target.name,
            target_kind: target.kind,
            crate_types: target.crate_types,
            source: normalized_relative(
                &metadata.workspace_root,
                &target.src_path,
                "target source",
            )?,
            edition: target.edition,
            doc: target.doc,
            doctest: target.doctest,
            target_test: target.test,
            profile_test: profile.test,
            opt_level: profile.opt_level,
            debuginfo: serde_json::to_string(&profile.debuginfo)?,
            debug_assertions: profile.debug_assertions,
            overflow_checks: profile.overflow_checks,
            features,
            filenames,
            executable,
        });
    }
    let mut workspace_packages: Vec<_> = workspace
        .values()
        .map(|package| package.name.clone())
        .collect();
    workspace_packages.sort();
    ensure!(
        !artifacts.is_empty(),
        "Cargo messages contain no workspace artifacts"
    );
    let inventory = Inventory {
        schema: SCHEMA,
        workspace_packages,
        artifacts: artifacts.into_iter().collect(),
    };
    for package in workspace.values() {
        for target in package.targets.iter().filter(|target| target.test) {
            ensure!(
                inventory.artifacts.iter().any(|artifact| {
                    artifact.package == package.name
                        && artifact.target_name == target.name
                        && artifact.target_kind == target.kind
                        && is_runnable(artifact)
                }),
                "Cargo inventory omits runnable target {} {}",
                package.name,
                target.name
            );
        }
    }
    Ok(inventory)
}

pub fn write_inventory(
    metadata_path: &Path,
    messages: &Path,
    target_dir: &Path,
    output: &Path,
) -> Result<()> {
    let inventory = artifact_inventory(&metadata(metadata_path)?, messages, target_dir)?;
    write_json(output, &inventory)
}

pub fn write_selection(inventory_path: &Path, packages: &[String], output: &Path) -> Result<()> {
    let inventory: Inventory = read_json(inventory_path)?;
    ensure!(inventory.schema == SCHEMA, "unsupported inventory schema");
    let mut packages = packages.to_vec();
    packages.sort();
    packages.dedup();
    ensure!(!packages.is_empty(), "coverage shard has no packages");
    for package in &packages {
        ensure!(
            inventory.workspace_packages.binary_search(package).is_ok(),
            "coverage shard names unknown package {package}"
        );
    }

    let selected: Vec<_> = inventory
        .artifacts
        .iter()
        .filter(|artifact| packages.binary_search(&artifact.package).is_ok())
        .cloned()
        .collect();
    validate_selection(&inventory, &selected, &packages)?;

    write_json(
        output,
        &Selection {
            schema: SCHEMA,
            inventory_sha256: digest_json(&inventory)?,
            packages,
            artifacts: selected,
        },
    )
}

fn is_runnable(artifact: &Artifact) -> bool {
    artifact.profile_test && artifact.executable.is_some()
}

fn validate_selection(
    inventory: &Inventory,
    selected: &[Artifact],
    packages: &[String],
) -> Result<()> {
    let selected_set: BTreeSet<_> = selected.iter().cloned().collect();
    ensure!(
        selected_set.len() == selected.len(),
        "selected inventory contains duplicate artifacts"
    );
    for artifact in &selected_set {
        ensure!(
            packages.binary_search(&artifact.package).is_ok(),
            "selected inventory includes artifact outside its shard: {} {}",
            artifact.package,
            artifact.target_name
        );
    }
    for package in packages {
        let required: BTreeSet<_> = inventory
            .artifacts
            .iter()
            .filter(|artifact| artifact.package == *package)
            .cloned()
            .collect();
        ensure!(
            !required.is_empty(),
            "full inventory omits package {package}"
        );
        ensure!(
            required.is_subset(&selected_set),
            "selected inventory omits an artifact for {package}"
        );
        ensure!(
            required.iter().any(is_runnable),
            "package {package} has no runnable Cargo test artifact"
        );
    }
    let expected: BTreeSet<_> = inventory
        .artifacts
        .iter()
        .filter(|artifact| packages.binary_search(&artifact.package).is_ok())
        .cloned()
        .collect();
    ensure!(
        selected_set == expected,
        "selected inventory differs from the full package artifacts"
    );
    Ok(())
}

fn runnable_artifacts(inventory_path: &Path, selection_path: &Path) -> Result<Vec<Artifact>> {
    let inventory: Inventory = read_json(inventory_path)?;
    let selection: Selection = read_json(selection_path)?;
    ensure!(
        inventory.schema == SCHEMA && selection.schema == SCHEMA,
        "unsupported coverage schema"
    );
    ensure!(
        selection.inventory_sha256 == digest_json(&inventory)?,
        "selection names another inventory"
    );
    validate_selection(&inventory, &selection.artifacts, &selection.packages)?;
    let runnable: Vec<_> = inventory
        .artifacts
        .into_iter()
        .filter(is_runnable)
        .collect();
    ensure!(
        !runnable.is_empty(),
        "coverage shard has no test executables"
    );
    for artifact in &runnable {
        let supported = artifact.target_kind.len() == 1
            && matches!(artifact.target_kind[0].as_str(), "lib" | "bin" | "test");
        ensure!(
            supported,
            "unsupported Cargo test target semantics for {} {}: {:?}",
            artifact.package,
            artifact.target_name,
            artifact.target_kind
        );
    }
    Ok(runnable)
}

pub struct RunnerConfigOptions<'a> {
    pub root: &'a Path,
    pub host: &'a str,
    pub helper: &'a Path,
    pub inventory: &'a Path,
    pub selection: &'a Path,
    pub target_dir: &'a Path,
    pub ledger: &'a Path,
    pub diagnostics: &'a Path,
    /// Hosted job start, in Unix seconds, recorded by the job's first step.
    pub job_started: u64,
    /// The hosted job's `timeout-minutes` limit.
    pub job_minutes: u64,
    pub output: &'a Path,
}

fn unix_now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock precedes the Unix epoch")?
        .as_secs())
}

/// Place the shard's test deadline strictly inside its hosted job limit, so a
/// stalled test fails inside the job with evidence instead of being cancelled.
pub fn shard_deadline(job_started: u64, job_minutes: u64) -> Result<u64> {
    let job = job_minutes
        .checked_mul(60)
        .context("coverage job limit overflows")?;
    ensure!(
        job > EVIDENCE_RESERVE.as_secs(),
        "coverage job limit of {job_minutes} minutes leaves no time inside the {}-second evidence reserve",
        EVIDENCE_RESERVE.as_secs()
    );
    job_started
        .checked_add(job - EVIDENCE_RESERVE.as_secs())
        .context("coverage deadline overflows")
}

pub fn write_runner_config(options: &RunnerConfigOptions<'_>) -> Result<()> {
    let RunnerConfigOptions {
        root,
        host,
        helper,
        inventory: inventory_path,
        selection: selection_path,
        target_dir,
        ledger,
        diagnostics,
        job_started,
        job_minutes,
        output,
    } = options;
    ensure!(root.is_absolute(), "coverage root must be absolute");
    ensure!(helper.is_absolute(), "coverage helper must be absolute");
    ensure!(
        inventory_path.is_absolute() && selection_path.is_absolute(),
        "coverage manifests must be absolute"
    );
    ensure!(target_dir.is_absolute(), "coverage target must be absolute");
    ensure!(ledger.is_absolute(), "coverage ledger must be absolute");
    ensure!(
        diagnostics.is_absolute() && fs::symlink_metadata(diagnostics)?.file_type().is_dir(),
        "coverage diagnostics must be an absolute directory"
    );
    let now = unix_now()?;
    ensure!(
        *job_started <= now,
        "coverage job start {job_started} is in the future"
    );
    let deadline = shard_deadline(*job_started, *job_minutes)?;
    ensure!(
        deadline > now,
        "coverage shard deadline {deadline} passed before its tests started"
    );
    ensure!(
        !host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid Cargo host target"
    );
    runnable_artifacts(inventory_path, selection_path)?;
    let strings = [
        helper,
        root,
        target_dir,
        inventory_path,
        selection_path,
        ledger,
        diagnostics,
    ]
    .map(|path| {
        path.to_str()
            .context("Cargo runner configuration path is not UTF-8")
    });
    let [
        helper,
        root,
        target_dir,
        inventory,
        selection,
        ledger,
        diagnostics,
    ] = strings;
    let runner = vec![
        helper?.to_owned(),
        "coverage".to_owned(),
        "dispatch".to_owned(),
        "--root".to_owned(),
        root?.to_owned(),
        "--target-dir".to_owned(),
        target_dir?.to_owned(),
        "--inventory".to_owned(),
        inventory?.to_owned(),
        "--selection".to_owned(),
        selection?.to_owned(),
        "--ledger".to_owned(),
        ledger?.to_owned(),
        "--diagnostics".to_owned(),
        diagnostics?.to_owned(),
        "--deadline".to_owned(),
        deadline.to_string(),
        "--".to_owned(),
    ];
    let mut host_table = toml::map::Map::new();
    host_table.insert(
        "runner".to_owned(),
        toml::Value::Array(runner.into_iter().map(toml::Value::String).collect()),
    );
    let mut targets = toml::map::Map::new();
    targets.insert((*host).to_owned(), toml::Value::Table(host_table));
    let mut document = toml::map::Map::new();
    document.insert("target".to_owned(), toml::Value::Table(targets));
    let bytes = toml::to_string(&toml::Value::Table(document))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(bytes.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn append_runner_record(ledger: &Path, record: &RunnerRecord) -> Result<()> {
    let current = match fs::symlink_metadata(ledger) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_file(),
                "coverage runner ledger is not a regular file"
            );
            metadata.len()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    ensure!(
        current
            .checked_add(bytes.len() as u64)
            .is_some_and(|total| total <= RUNNER_LEDGER_LIMIT),
        "coverage runner ledger is too large"
    );
    let mut file = OpenOptions::new().create(true).append(true).open(ledger)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

pub struct DispatchOptions<'a> {
    pub root: &'a Path,
    pub inventory: &'a Path,
    pub selection: &'a Path,
    pub target_dir: &'a Path,
    pub ledger: &'a Path,
    pub diagnostics: &'a Path,
    /// Unix-seconds deadline written into the runner configuration.
    pub deadline: u64,
    pub executable: &'a Path,
    pub args: &'a [OsString],
}

/// Observed libtest progress from one test executable's standard output.
///
/// Libtest prints a completion line for each finished test and, while running
/// concurrently, a notice for each test that has run for over 60 seconds. A
/// single-threaded harness prints `test name ... ` before the result instead.
#[derive(Debug, Default)]
struct LibtestProgress {
    pending: Vec<u8>,
    pending_overflowed: bool,
    long_running: Vec<String>,
    recent: std::collections::VecDeque<String>,
    completed: usize,
}

impl LibtestProgress {
    fn observe(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                let line = std::mem::take(&mut self.pending);
                if !std::mem::take(&mut self.pending_overflowed) {
                    self.line(String::from_utf8_lossy(&line).trim_end_matches('\r'));
                }
            } else if self.pending.len() < PENDING_LINE_LIMIT {
                self.pending.push(byte);
            } else {
                self.pending_overflowed = true;
            }
        }
    }

    fn line(&mut self, line: &str) {
        let Some(rest) = line.strip_prefix("test ") else {
            return;
        };
        if let Some((name, _)) = rest.split_once(" has been running for ") {
            if !self.long_running.iter().any(|running| running == name) {
                self.long_running.push(name.to_owned());
            }
        } else if let Some((name, outcome)) = rest.split_once(" ... ")
            && !outcome.is_empty()
        {
            self.long_running.retain(|running| running != name);
            self.completed += 1;
            if self.recent.len() == RECENT_RESULT_LIMIT {
                self.recent.pop_front();
            }
            self.recent.push_back(line.to_owned());
        }
    }

    /// Tests that started but have no completion line.
    fn unfinished(&self) -> Vec<String> {
        let mut unfinished = self.long_running.clone();
        if !self.pending_overflowed
            && let Some(name) = std::str::from_utf8(&self.pending)
                .ok()
                .and_then(|line| line.strip_prefix("test "))
                .and_then(|rest| rest.strip_suffix(" ... "))
            && !unfinished.iter().any(|running| running == name)
        {
            unfinished.push(name.to_owned());
        }
        unfinished
    }
}

/// Evidence written when a test executable reaches the shard deadline.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct StallReport {
    schema: u32,
    executable: String,
    package: String,
    target_name: String,
    deadline_unix: u64,
    observed_unix: u64,
    /// Tests libtest reported as running without a completion line.
    unfinished_tests: Vec<String>,
    /// The latest completion lines, oldest first.
    recent_results: Vec<String>,
    completed_results: usize,
    process_sample: Option<String>,
    termination: String,
    cleanup: String,
    /// Unix only: the owned process group's presence, observed after the
    /// root was reaped. It is never observed before reaping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    presence_after_reap: Option<String>,
    output: String,
    stdout_log: String,
}

/// Owned test process tree driven by the coverage runner.
trait TestProcess {
    async fn wait(&mut self, timeout: Duration) -> std::io::Result<ExitStatus>;
    /// Diagnostic resource sample taken before termination.
    fn sample(&self) -> Option<String>;
    /// Terminate the retained tree, never a numeric identity.
    fn terminate(&mut self) -> std::io::Result<()>;
    /// The tree's residual presence, available only once its root is reaped.
    fn presence_after_reap(&self) -> Option<String> {
        None
    }
}

struct StallEvidence {
    progress: LibtestProgress,
    sample: Option<String>,
    termination: String,
    cleanup: String,
    presence_after_reap: Option<String>,
    output: String,
}

enum Supervision {
    Exited(ExitStatus),
    Stalled(Box<StallEvidence>),
}

/// Keep the bounded log copy of one relayed chunk.
async fn log_chunk(
    file: &mut tokio::fs::File,
    chunk: &[u8],
    logged: &mut u64,
    truncated: &mut bool,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let room = TEST_LOG_LIMIT.saturating_sub(*logged);
    let kept = (chunk.len() as u64).min(room) as usize;
    if kept > 0 {
        file.write_all(&chunk[..kept]).await?;
        *logged += kept as u64;
    }
    if kept < chunk.len() && !*truncated {
        *truncated = true;
        file.write_all(b"\n[coverage runner: log truncated at its byte limit]\n")
            .await?;
    }
    // A stalled relay is dropped; flush so its log keeps everything seen.
    file.flush().await
}

/// Relay the test's stdout to the job log while keeping a bounded copy and
/// observing libtest progress. Returns `complete`, or the errors observed.
///
/// A log or relay failure never ends the relay: the test's stdout pipe keeps
/// being drained, so the test is not broken by its own next write, and the
/// remaining destination keeps receiving output. Only a read failure ends it.
async fn relay_output<R, W>(
    mut output: R,
    mut relay: W,
    log: &Path,
    progress: &mut LibtestProgress,
) -> String
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut errors = Vec::new();
    let mut file = match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(log)
        .await
    {
        Ok(file) => Some(file),
        Err(error) => {
            errors.push(format!("log create {} failed: {error}", log.display()));
            None
        }
    };
    let mut relaying = true;
    let mut logged = 0_u64;
    let mut truncated = false;
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let length = match output.read(&mut buffer).await {
            Ok(0) => break,
            Ok(length) => length,
            Err(error) => {
                errors.push(format!("output read failed: {error}"));
                break;
            }
        };
        let chunk = &buffer[..length];
        progress.observe(chunk);
        if relaying {
            let relayed = match relay.write_all(chunk).await {
                Ok(()) => relay.flush().await,
                Err(error) => Err(error),
            };
            if let Err(error) = relayed {
                errors.push(format!("job log relay failed: {error}"));
                relaying = false;
            }
        }
        if let Some(open) = file.as_mut()
            && let Err(error) = log_chunk(open, chunk, &mut logged, &mut truncated).await
        {
            errors.push(format!("log write failed: {error}"));
            file = None;
        }
    }
    if let Some(file) = file
        && let Err(error) = file.sync_all().await
    {
        errors.push(format!("log sync failed: {error}"));
    }
    if errors.is_empty() {
        "complete".to_owned()
    } else {
        errors.join("; ")
    }
}

/// Wait for the owned tree until the shard deadline while relaying its output.
/// At the deadline, sample, terminate and await bounded cleanup, then return the
/// observed progress. Output draining is bounded in both outcomes: a process
/// outside the owned tree may still hold the pipe after the tree is quiescent.
async fn supervise<P, R, W>(
    process: &mut P,
    output: R,
    relay: W,
    log: PathBuf,
    remaining: Duration,
    cleanup_bound: Duration,
) -> Result<Supervision>
where
    P: TestProcess,
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let deadline = tokio::time::Instant::now() + remaining;
    let mut progress = LibtestProgress::default();
    let (waited, sample, termination, cleanup, presence_after_reap, output) = {
        let mut relay = std::pin::pin!(relay_output(output, relay, &log, &mut progress));
        let mut relayed = None;
        let waited = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            tokio::select! {
                outcome = &mut relay, if relayed.is_none() => relayed = Some(outcome),
                waited = process.wait(remaining) => break waited,
            }
        };
        let (sample, termination, cleanup, presence_after_reap) = if waited.is_err() {
            let sample = process.sample();
            let termination = format!("{:?}", process.terminate());
            let cleanup = format!("{:?}", process.wait(cleanup_bound).await);
            // Recorded only after the cleanup wait has had its chance to reap.
            (sample, termination, cleanup, process.presence_after_reap())
        } else {
            (None, String::new(), String::new(), None)
        };
        let output = match relayed {
            Some(outcome) => outcome,
            None => match tokio::time::timeout(cleanup_bound, &mut relay).await {
                Ok(outcome) => outcome,
                Err(_) => format!(
                    "stopped after {cleanup_bound:?}: a process outside the owned tree still holds the output"
                ),
            },
        };
        (
            waited,
            sample,
            termination,
            cleanup,
            presence_after_reap,
            output,
        )
    };
    match waited {
        Ok(status) => {
            if output != "complete" {
                eprintln!("coverage runner output relay: {output}");
            }
            Ok(Supervision::Exited(status))
        }
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            Ok(Supervision::Stalled(Box::new(StallEvidence {
                progress,
                sample,
                termination,
                cleanup,
                presence_after_reap,
                output,
            })))
        }
        Err(error) => bail!(
            "Cargo test process did not settle: {error}; termination={termination}; cleanup={cleanup}; output={output}"
        ),
    }
}

fn diagnostic_name(executable: &str) -> String {
    executable
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub async fn dispatch_test(options: &DispatchOptions<'_>) -> Result<Option<ExitStatus>> {
    let DispatchOptions {
        root,
        inventory: inventory_path,
        selection: selection_path,
        target_dir,
        ledger,
        diagnostics,
        deadline,
        executable,
        args,
    } = options;
    ensure!(root.is_absolute(), "coverage root must be absolute");
    ensure!(
        inventory_path.is_absolute() && selection_path.is_absolute(),
        "coverage manifests must be absolute"
    );
    ensure!(target_dir.is_absolute(), "coverage target must be absolute");
    ensure!(ledger.is_absolute(), "coverage ledger must be absolute");
    ensure!(
        diagnostics.is_absolute() && fs::symlink_metadata(diagnostics)?.file_type().is_dir(),
        "coverage diagnostics must be an absolute directory"
    );
    let executable_path = if executable.is_absolute() {
        executable.to_path_buf()
    } else {
        std::env::current_dir()?.join(executable)
    };
    ensure!(
        fs::symlink_metadata(&executable_path)?
            .file_type()
            .is_file(),
        "Cargo runner executable is not a regular file"
    );
    let executable = normalized_relative(target_dir, &executable_path, "runner executable")?;
    let artifacts = runnable_artifacts(inventory_path, selection_path)?;
    let artifact = artifacts
        .iter()
        .find(|artifact| artifact.executable.as_deref() == Some(executable.as_str()))
        .with_context(|| format!("Cargo invoked unknown test executable {executable}"))?;
    ensure!(
        artifacts
            .iter()
            .filter(|candidate| candidate.executable.as_deref() == Some(executable.as_str()))
            .count()
            == 1,
        "Cargo test executable identity is ambiguous"
    );
    let cwd = normalized_relative(root, &std::env::current_dir()?, "runner directory")?;
    ensure!(
        cwd == artifact.package_root,
        "Cargo ran {} {} from {cwd}, expected {}",
        artifact.package,
        artifact.target_name,
        artifact.package_root
    );
    let args = args
        .iter()
        .map(|arg| {
            arg.to_str()
                .context("Cargo runner argument is not UTF-8")
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        args.is_empty(),
        "Cargo supplied unexpected libtest arguments"
    );
    let selection: Selection = read_json(selection_path)?;
    let selected = selection.packages.binary_search(&artifact.package).is_ok();
    if !selected {
        append_runner_record(
            ledger,
            &RunnerRecord {
                schema: SCHEMA,
                executable,
                action: "omit".to_owned(),
                cwd,
                args,
                success: true,
                status_code: Some(0),
            },
        )?;
        return Ok(None);
    }
    let now = unix_now()?;
    if now >= *deadline {
        append_runner_record(
            ledger,
            &RunnerRecord {
                schema: SCHEMA,
                executable: executable.clone(),
                action: "deadline".to_owned(),
                cwd,
                args,
                success: false,
                status_code: None,
            },
        )?;
        bail!("coverage shard deadline {deadline} passed before starting {executable}");
    }
    let remaining = Duration::from_secs(deadline - now);
    let log = diagnostics.join(format!("{}.stdout.log", diagnostic_name(&executable)));
    #[cfg(windows)]
    let supervision = {
        use kuru_platform::windows::process::{
            Lifetime, NativeChild, NativeSpawnSpec, StandardStream, Stdio, inherited_stdio,
            sample_process,
        };

        struct Native(NativeChild);
        impl TestProcess for Native {
            async fn wait(&mut self, timeout: Duration) -> std::io::Result<ExitStatus> {
                self.0.wait(timeout).await
            }
            fn sample(&self) -> Option<String> {
                Some(
                    match self
                        .0
                        .duplicate_diagnostic_handle()
                        .and_then(|handle| sample_process(&handle))
                    {
                        Ok(sample) => format!(
                            "kernel_time={:?}; user_time={:?}; working_set_bytes={}",
                            sample.kernel_time, sample.user_time, sample.working_set_bytes
                        ),
                        Err(error) => format!("sample failed: {error}"),
                    },
                )
            }
            fn terminate(&mut self) -> std::io::Result<()> {
                self.0.terminate()
            }
        }

        let mut spec = NativeSpawnSpec::new(executable_path, std::env::current_dir()?);
        // Only the verified memory test artifact exercises an owner that must
        // outlive its starter. Its test process remains in a kill-on-close Job,
        // but that Job permits the service's explicit native breakaway. All
        // other test artifacts retain the ordinary owned Job policy.
        if needs_independent_service_lifetime(artifact) {
            spec.lifetime = Lifetime::FixtureBreakawayJob;
        }
        spec.args = args.iter().map(OsString::from).collect();
        spec.environment = std::env::vars_os().collect();
        spec.stdin = inherited_stdio(StandardStream::Input)?;
        // The runner relays stdout to the job log and observes libtest progress.
        spec.stdout = Stdio::Pipe;
        spec.stderr = inherited_stdio(StandardStream::Error)?;
        let mut child = spec.spawn().await?;
        let output = child
            .take_stdout()
            .context("Cargo test process has no stdout pipe")?;
        let mut child = Native(child);
        supervise(
            &mut child,
            output,
            tokio::io::stdout(),
            log.clone(),
            remaining,
            RUNNER_CLEANUP_TIMEOUT,
        )
        .await?
    };
    #[cfg(unix)]
    let supervision = {
        // The test inherits this runner's complete environment, including the
        // LLVM_PROFILE_FILE destination, working directory, stdin and stderr.
        let mut command = std::process::Command::new(executable_path);
        command
            .args(&args)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        supervise_group(
            command,
            tokio::io::stdout(),
            log.clone(),
            remaining,
            RUNNER_CLEANUP_TIMEOUT,
        )
        .await?
    };
    let status = match supervision {
        Supervision::Exited(status) => status,
        Supervision::Stalled(evidence) => {
            append_runner_record(
                ledger,
                &RunnerRecord {
                    schema: SCHEMA,
                    executable: executable.clone(),
                    action: "run".to_owned(),
                    cwd,
                    args,
                    success: false,
                    status_code: None,
                },
            )?;
            let report = stall_report(artifact, &executable, *deadline, evidence, &log)?;
            let path = diagnostics.join(format!("{}.stall.json", diagnostic_name(&executable)));
            write_json(&path, &report)?;
            bail!(
                "coverage shard deadline reached while {executable} was running; unfinished tests: {:?}; latest results: {:?}; evidence: {}",
                report.unfinished_tests,
                report.recent_results.last(),
                path.display()
            );
        }
    };
    append_runner_record(
        ledger,
        &RunnerRecord {
            schema: SCHEMA,
            executable,
            action: "run".to_owned(),
            cwd,
            args,
            success: status.success(),
            status_code: status.code(),
        },
    )?;
    Ok(Some(status))
}

fn stall_report(
    artifact: &Artifact,
    executable: &str,
    deadline: u64,
    evidence: Box<StallEvidence>,
    log: &Path,
) -> Result<StallReport> {
    let StallEvidence {
        progress,
        sample,
        termination,
        cleanup,
        presence_after_reap,
        output,
    } = *evidence;
    Ok(StallReport {
        schema: SCHEMA,
        executable: executable.to_owned(),
        package: artifact.package.clone(),
        target_name: artifact.target_name.clone(),
        deadline_unix: deadline,
        observed_unix: unix_now()?,
        unfinished_tests: progress.unfinished(),
        recent_results: progress.recent.iter().cloned().collect(),
        completed_results: progress.completed,
        process_sample: sample,
        termination,
        cleanup,
        presence_after_reap,
        output,
        stdout_log: log
            .file_name()
            .and_then(OsStr::to_str)
            .context("stdout log lacks a UTF-8 name")?
            .to_owned(),
    })
}

/// Poll interval for the owned Unix process group's non-reaping observations.
#[cfg(unix)]
const GROUP_POLL: Duration = Duration::from_millis(20);

/// One test executable anchored to a fresh Unix process group.
///
/// Waiting is driven by the owner's phase, so the same path serves a normal
/// exit and the cleanup after [`TestProcess::terminate`]. An exited root is not
/// reaped directly: the owner consumes its one group-then-root transition first
/// (the unreaped root still pins its identity, so remaining group members are
/// signalled and the root's own exit status is preserved), then reaps the exact
/// root and only afterwards observes whether the group is absent.
#[cfg(unix)]
struct GroupProcess {
    owner: kuru_platform::unix::OwnedProcessGroup,
    /// Last non-reaping root observation, for the stall sample.
    observed: String,
    presence: Option<String>,
    /// Bound for the transition, reap and absence confirmation after exit.
    settle_bound: Duration,
}

#[cfg(unix)]
impl GroupProcess {
    fn spawn(command: std::process::Command, settle_bound: Duration) -> std::io::Result<Self> {
        Ok(Self {
            owner: kuru_platform::unix::OwnedProcessGroup::spawn(command)?,
            observed: "not observed".to_owned(),
            presence: None,
            settle_bound,
        })
    }

    fn take_stdout(&mut self) -> std::io::Result<tokio::process::ChildStdout> {
        tokio::process::ChildStdout::from_std(self.owner.take_stdout()?)
    }

    /// Consume the transition, reap the exact root, then confirm absence.
    async fn settle(&mut self) -> std::io::Result<ExitStatus> {
        use kuru_platform::unix::{GroupPresence, Reap, Termination};

        let bound = self.settle_bound;
        let limit = tokio::time::Instant::now() + bound;
        let expired = |what: &str| {
            std::io::Error::other(format!("owned test process group {what} within {bound:?}"))
        };
        loop {
            match self.owner.terminate_before_reap() {
                Termination::Signalled(_) | Termination::InvalidPhase => break,
                Termination::Interrupted => {}
                Termination::Disarmed(reason) => return Err(disarmed(reason)),
            }
            if tokio::time::Instant::now() >= limit {
                return Err(expired("could not be signalled"));
            }
            tokio::time::sleep(GROUP_POLL).await;
        }
        let status = loop {
            match self.owner.reap_if_exited() {
                Reap::Reaped(status) => break status,
                Reap::NotExited | Reap::Interrupted => {}
                Reap::Disarmed(reason) => return Err(disarmed(reason)),
                Reap::InvalidPhase => {
                    return Err(std::io::Error::other(
                        "owned test process reap preceded its transition",
                    ));
                }
            }
            if tokio::time::Instant::now() >= limit {
                return Err(expired("root was not reaped"));
            }
            tokio::time::sleep(GROUP_POLL).await;
        };
        loop {
            let presence = self.owner.presence_after_reap();
            self.presence = Some(format!("{presence:?}"));
            match presence {
                GroupPresence::Absent => return Ok(status),
                GroupPresence::Present | GroupPresence::PermissionDenied => {}
                GroupPresence::ObservationError(_) | GroupPresence::InvalidPhase => {
                    return Err(std::io::Error::other(format!(
                        "owned test process group absence could not be observed: {presence:?}"
                    )));
                }
            }
            if tokio::time::Instant::now() >= limit {
                return Err(expired("remained present after its root was reaped"));
            }
            tokio::time::sleep(GROUP_POLL).await;
        }
    }
}

/// Start one test command in a fresh owned process group and supervise it.
/// The command's stdout must be piped; everything else is the caller's.
#[cfg(unix)]
async fn supervise_group<W>(
    command: std::process::Command,
    relay: W,
    log: PathBuf,
    remaining: Duration,
    cleanup_bound: Duration,
) -> Result<Supervision>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut child = GroupProcess::spawn(command, cleanup_bound)?;
    let output = match child.take_stdout() {
        Ok(output) => output,
        Err(error) => {
            let termination = child.terminate();
            let cleanup = child.wait(cleanup_bound).await;
            bail!(
                "Cargo test process has no stdout pipe: {error}; termination={termination:?}; cleanup={cleanup:?}"
            );
        }
    };
    supervise(&mut child, output, relay, log, remaining, cleanup_bound).await
}

#[cfg(unix)]
fn disarmed(reason: kuru_platform::unix::DisarmReason) -> std::io::Error {
    std::io::Error::other(format!(
        "owned test process group authority was disarmed: {reason:?}"
    ))
}

#[cfg(unix)]
impl TestProcess for GroupProcess {
    async fn wait(&mut self, timeout: Duration) -> std::io::Result<ExitStatus> {
        use kuru_platform::unix::RootState;

        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let state = self.owner.root_state();
            self.observed = format!("{state:?}");
            match state {
                RootState::Exited => return self.settle().await,
                RootState::Reaped(status) => return Ok(status),
                RootState::Running | RootState::Interrupted => {}
                RootState::Disarmed(reason) => return Err(disarmed(reason)),
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "owned test process group did not exit",
                ));
            }
            tokio::time::sleep(GROUP_POLL.min(deadline - now)).await;
        }
    }

    fn sample(&self) -> Option<String> {
        Some(format!("root_state={}", self.observed))
    }

    fn terminate(&mut self) -> std::io::Result<()> {
        use kuru_platform::unix::Termination;

        match self.owner.terminate_before_reap() {
            // The transition may already have been consumed by a settling exit.
            Termination::Signalled(_) | Termination::InvalidPhase => Ok(()),
            Termination::Interrupted => Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "owned test process group observation was interrupted",
            )),
            Termination::Disarmed(reason) => Err(disarmed(reason)),
        }
    }

    fn presence_after_reap(&self) -> Option<String> {
        self.presence.clone()
    }
}

#[cfg(windows)]
fn needs_independent_service_lifetime(artifact: &Artifact) -> bool {
    artifact.package == "kuru-memory"
        && artifact.package_root == "packages/kuru-memory"
        && artifact.target_kind.len() == 1
        && artifact.target_kind[0] == "lib"
}

pub fn validate_run_ledger(
    inventory_path: &Path,
    selection_path: &Path,
    ledger: &Path,
) -> Result<()> {
    let artifacts = runnable_artifacts(inventory_path, selection_path)?;
    let selection: Selection = read_json(selection_path)?;
    ensure!(
        fs::symlink_metadata(ledger)?.file_type().is_file(),
        "coverage runner ledger is not a regular file"
    );
    ensure!(
        fs::symlink_metadata(ledger)?.len() <= RUNNER_LEDGER_LIMIT,
        "coverage runner ledger is too large"
    );
    let reader = BufReader::new(File::open(ledger)?);
    let mut records = BTreeMap::new();
    for (index, line) in reader.lines().enumerate() {
        ensure!(
            index < artifacts.len(),
            "Cargo runner ledger has extra records"
        );
        let record: RunnerRecord = serde_json::from_str(&line?)?;
        ensure!(record.schema == SCHEMA, "unsupported runner ledger schema");
        ensure!(
            record.args.is_empty(),
            "Cargo supplied unexpected libtest arguments"
        );
        ensure!(
            records.insert(record.executable.clone(), record).is_none(),
            "Cargo invoked a test executable more than once"
        );
    }
    ensure!(
        records.len() == artifacts.len(),
        "Cargo runner ledger omits test executables"
    );
    for artifact in artifacts {
        let executable = artifact.executable.as_deref().unwrap();
        let record = records
            .get(executable)
            .with_context(|| format!("Cargo did not invoke {executable}"))?;
        let expected = if selection.packages.binary_search(&artifact.package).is_ok() {
            "run"
        } else {
            "omit"
        };
        ensure!(
            record.action == expected,
            "wrong runner action for {executable}"
        );
        ensure!(
            record.success && record.status_code == Some(0),
            "Cargo runner did not complete {executable} successfully"
        );
        ensure!(
            record.cwd == artifact.package_root,
            "wrong Cargo working directory for {executable}"
        );
    }
    Ok(())
}

async fn checked_output(root: &Path, program: &OsStr, args: &[&str]) -> Result<String> {
    let mut child = command::rooted(root, program);
    child.args(args);
    let output = command::bounded_output(&mut child, COMMAND_TIMEOUT, COMMAND_OUTPUT_LIMIT)
        .await
        .with_context(|| format!("run {}", Path::new(program).display()))?;
    ensure!(
        output.status.success(),
        "{} {:?} failed: {}",
        Path::new(program).display(),
        args,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

async fn identity(root: &Path, expected_source: &str, llvm_cov: &Path) -> Result<ReceiptIdentity> {
    let source = checked_output(root, OsStr::new("git"), &["rev-parse", "HEAD"]).await?;
    ensure!(
        source == expected_source,
        "coverage source is {source}, expected {expected_source}"
    );
    let tree = checked_output(root, OsStr::new("git"), &["rev-parse", "HEAD^{tree}"]).await?;
    let status = checked_output(
        root,
        OsStr::new("git"),
        &["status", "--porcelain=v1", "--untracked-files=no"],
    )
    .await?;
    ensure!(
        status.is_empty(),
        "coverage source has modified tracked files"
    );
    let rustc = checked_output(root, OsStr::new("rustc"), &["-vV"]).await?;
    let target = rustc
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .context("rustc identity omits host target")?
        .to_owned();
    let cargo = checked_output(root, OsStr::new("cargo"), &["-vV"]).await?;
    let cargo_llvm_cov =
        checked_output(root, llvm_cov.as_os_str(), &["llvm-cov", "--version"]).await?;
    ensure!(
        cargo_llvm_cov == LLVM_COV_VERSION,
        "expected {LLVM_COV_VERSION}, got {cargo_llvm_cov}"
    );
    let lock = read_bounded(&root.join("Cargo.lock"), JSON_LIMIT)?;
    Ok(ReceiptIdentity {
        source,
        tree,
        cargo_lock_sha256: archive::digest(&lock),
        rustc,
        cargo,
        target,
        cargo_llvm_cov,
        target_os: std::env::consts::OS.to_owned(),
    })
}

#[derive(Clone, Debug)]
struct ReceiptIdentity {
    source: String,
    tree: String,
    cargo_lock_sha256: String,
    rustc: String,
    cargo: String,
    target: String,
    cargo_llvm_cov: String,
    target_os: String,
}

pub async fn verify_source(root: &Path, expected_source: &str, llvm_cov: &Path) -> Result<()> {
    identity(root, expected_source, llvm_cov).await.map(drop)
}

fn hash_copy(source: &Path, destination: &Path) -> Result<ProfileReceipt> {
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        metadata.file_type().is_file(),
        "profile {} is not a regular file",
        source.display()
    );
    ensure!(
        metadata.len() <= PROFILE_LIMIT,
        "profile {} is too large",
        source.display()
    );
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = input.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        bytes = bytes
            .checked_add(length as u64)
            .context("profile size overflow")?;
        ensure!(
            bytes <= PROFILE_LIMIT,
            "profile {} grew too large",
            source.display()
        );
        hasher.update(&buffer[..length]);
        output.write_all(&buffer[..length])?;
    }
    ensure!(bytes > 0, "profile {} is empty", source.display());
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(ProfileReceipt {
        name: destination
            .file_name()
            .and_then(OsStr::to_str)
            .context("profile destination lacks a UTF-8 name")?
            .to_owned(),
        bytes,
        sha256,
    })
}

fn copy_checked_profile(
    source: &Path,
    destination: &Path,
    expected: &ProfileReceipt,
) -> Result<()> {
    let copied = hash_copy(source, destination)?;
    ensure!(
        copied.bytes == expected.bytes && copied.sha256 == expected.sha256,
        "profile {} changed",
        expected.name
    );
    Ok(())
}

fn verify_profile(source: &Path, expected: &ProfileReceipt) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        metadata.file_type().is_file(),
        "profile {} is not a regular file",
        source.display()
    );
    ensure!(
        metadata.len() == expected.bytes,
        "profile {} changed",
        expected.name
    );
    let mut input = File::open(source)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = input.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        bytes = bytes
            .checked_add(length as u64)
            .context("profile size overflow")?;
        ensure!(
            bytes <= PROFILE_LIMIT,
            "profile {} grew too large",
            source.display()
        );
        hasher.update(&buffer[..length]);
    }
    let sha256: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        bytes == expected.bytes && sha256 == expected.sha256,
        "profile {} changed",
        expected.name
    );
    Ok(())
}

fn profile_sources(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.path().extension() == Some(OsStr::new("profraw")) {
            candidates.push(entry.path());
        }
    }
    candidates.sort();
    ensure!(
        candidates.len() <= PROFILE_COUNT_LIMIT,
        "coverage shard produced too many profiles"
    );
    let mut profiles = Vec::new();
    let mut total = 0_u64;
    for path in candidates {
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.file_type().is_file(),
            "profile {} is not a regular file",
            path.display()
        );
        ensure!(
            metadata.len() <= PROFILE_LIMIT,
            "profile {} is too large",
            path.display()
        );
        total = total
            .checked_add(metadata.len())
            .context("profile total size overflow")?;
        if metadata.len() > 0 {
            profiles.push(path);
        }
    }
    ensure!(
        total <= PROFILE_TOTAL_LIMIT,
        "coverage shard profiles exceed total size limit"
    );
    ensure!(
        !profiles.is_empty(),
        "coverage shard produced no nonempty raw profiles"
    );
    Ok(profiles)
}

fn ensure_no_profiles(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        ensure!(
            entry.path().extension() != Some(OsStr::new("profraw")),
            "aggregate target already contains raw profile {}",
            entry.path().display()
        );
    }
    Ok(())
}

pub fn discard_compile_profiles(directory: &Path) -> Result<usize> {
    let entries = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let mut removed = 0;
    for profile in entries {
        if profile.extension() != Some(OsStr::new("profraw")) {
            continue;
        }
        ensure!(
            fs::symlink_metadata(&profile)?.file_type().is_file(),
            "compile profile {} is not a regular file",
            profile.display()
        );
        fs::remove_file(&profile)?;
        removed += 1;
    }
    Ok(removed)
}

fn validate_profile_receipts(profiles: &[ProfileReceipt]) -> Result<()> {
    ensure!(!profiles.is_empty(), "coverage receipt has no raw profiles");
    ensure!(
        profiles.len() <= PROFILE_COUNT_LIMIT,
        "coverage receipt has too many raw profiles"
    );
    let mut names = BTreeSet::new();
    let mut total = 0_u64;
    for profile in profiles {
        let path = Path::new(&profile.name);
        ensure!(
            path.components().count() == 1
                && path.extension() == Some(OsStr::new("profraw"))
                && path.file_stem().is_some_and(|stem| !stem.is_empty()),
            "invalid raw profile name {}",
            profile.name
        );
        ensure!(
            names.insert(profile.name.as_str()),
            "duplicate raw profile name {}",
            profile.name
        );
        ensure!(
            profile.bytes > 0 && profile.bytes <= PROFILE_LIMIT,
            "invalid raw profile size for {}",
            profile.name
        );
        total = total
            .checked_add(profile.bytes)
            .context("profile receipt total size overflow")?;
    }
    ensure!(
        total <= PROFILE_TOTAL_LIMIT,
        "coverage receipt profiles exceed total size limit"
    );
    Ok(())
}

fn shard_packages(name: &str) -> Result<Vec<String>> {
    let (_, packages) = SHARDS
        .iter()
        .find(|(shard, _)| *shard == name)
        .with_context(|| format!("unknown coverage shard {name}"))?;
    let mut packages: Vec<_> = packages
        .iter()
        .map(|package| (*package).to_owned())
        .collect();
    packages.sort();
    Ok(packages)
}

pub struct ReceiptOptions<'a> {
    pub root: &'a Path,
    pub inventory: &'a Path,
    pub selection: &'a Path,
    pub ledger: &'a Path,
    pub profiles: &'a Path,
    pub shard: &'a str,
    pub run_attempt: &'a str,
    pub expected_source: &'a str,
    pub llvm_cov: &'a Path,
    pub output: &'a Path,
}

pub async fn write_receipt(options: &ReceiptOptions<'_>) -> Result<()> {
    let ReceiptOptions {
        root,
        inventory: inventory_path,
        selection: selection_path,
        ledger,
        profiles,
        shard,
        run_attempt,
        expected_source,
        llvm_cov,
        output,
    } = options;
    let attempt = canonical_attempt(run_attempt)
        .context("coverage run attempt must be a positive integer without leading zeros")?;
    let inventory: Inventory = read_json(inventory_path)?;
    let selection: Selection = read_json(selection_path)?;
    ensure!(
        inventory.schema == SCHEMA && selection.schema == SCHEMA,
        "unsupported coverage schema"
    );
    let packages = shard_packages(shard)?;
    ensure!(
        selection.packages == packages,
        "selection packages do not match shard {shard}"
    );
    let inventory_sha256 = digest_json(&inventory)?;
    ensure!(
        selection.inventory_sha256 == inventory_sha256,
        "selection names another inventory"
    );
    validate_selection(&inventory, &selection.artifacts, &packages)?;
    validate_run_ledger(inventory_path, selection_path, ledger)?;
    let runner_ledger = read_bounded(ledger, RUNNER_LEDGER_LIMIT)?;
    let observed = identity(root, expected_source, llvm_cov).await?;
    // The uploaded artifact root holds one `attempt-<n>` directory, so the
    // evidence names its attempt whether download-artifact extracts it into
    // the destination itself or into a directory named after the artifact.
    fs::create_dir(output).with_context(|| format!("create {}", output.display()))?;
    let output = &output.join(attempt_directory(attempt));
    fs::create_dir(output).with_context(|| format!("create {}", output.display()))?;
    let profile_output = output.join("profiles");
    fs::create_dir(&profile_output)?;
    let mut copied = Vec::new();
    for (index, source) in profile_sources(profiles)?.iter().enumerate() {
        copied.push(hash_copy(
            source,
            &profile_output.join(format!("{index:04}.profraw")),
        )?);
    }
    write_json(&output.join("inventory.json"), &inventory)?;
    write_json(&output.join("selection.json"), &selection)?;
    let mut ledger_output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("runner-ledger.jsonl"))?;
    ledger_output.write_all(&runner_ledger)?;
    write_json(
        &output.join("receipt.json"),
        &Receipt {
            schema: SCHEMA,
            shard: (*shard).to_owned(),
            run_attempt: (*run_attempt).to_owned(),
            packages,
            source: observed.source,
            tree: observed.tree,
            cargo_lock_sha256: observed.cargo_lock_sha256,
            rustc: observed.rustc,
            cargo: observed.cargo,
            target: observed.target,
            cargo_llvm_cov: observed.cargo_llvm_cov,
            instrumentation: INSTRUMENTATION.to_owned(),
            target_os: observed.target_os,
            inventory_sha256,
            selection_sha256: digest_json(&selection)?,
            runner_ledger_sha256: archive::digest(&runner_ledger),
            profiles: copied,
        },
    )
}

fn exact_entries(directory: &Path, expected: &[&str]) -> Result<()> {
    let actual: BTreeSet<_> = fs::read_dir(directory)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| std::io::Error::other("artifact has a non-UTF-8 name"))
        })
        .collect::<std::io::Result<_>>()?;
    let expected: BTreeSet<_> = expected.iter().map(|name| (*name).to_owned()).collect();
    ensure!(
        actual == expected,
        "unexpected artifact entries in {}: {actual:?}",
        directory.display()
    );
    Ok(())
}

fn canonical_attempt(text: &str) -> Option<u64> {
    let mut bytes = text.bytes();
    let first = bytes.next()?;
    ((b'1'..=b'9').contains(&first) && bytes.all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

const ATTEMPT_DIRECTORY: &str = "attempt-";

fn attempt_directory(attempt: u64) -> String {
    format!("{ATTEMPT_DIRECTORY}{attempt}")
}

/// Sorted names and paths of a directory's entries, which must all be directories.
fn directory_names(directory: &Path, shard: &str) -> Result<Vec<(String, PathBuf)>> {
    let mut entries = fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            let name = entry.file_name().into_string().map_err(|_| {
                anyhow::anyhow!("coverage shard {shard} artifact has a non-UTF-8 name")
            })?;
            ensure!(
                fs::symlink_metadata(entry.path())?.file_type().is_dir(),
                "coverage shard {shard} entry {name} is not a directory"
            );
            Ok((name, entry.path()))
        })
        .collect::<Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}

/// Parse the self-describing `attempt-<n>` evidence directory name.
fn evidence_attempt(shard: &str, name: &str) -> Result<u64> {
    name.strip_prefix(ATTEMPT_DIRECTORY)
        .and_then(canonical_attempt)
        .with_context(|| {
            format!("coverage shard {shard} evidence {name} is not a canonical attempt directory")
        })
}

/// Choose one shard's highest uploaded attempt.
///
/// Each uploaded artifact holds exactly one `attempt-<n>` directory. The pinned
/// download-artifact extracts a pattern download that matches one artifact
/// directly into the shard directory (`<shard>/attempt-<n>`), and one that
/// matches several into a directory per artifact name
/// (`<shard>/<name>/attempt-<n>`, where the name must carry the same attempt).
/// A mixture of the two layouts is refused. Failed shards upload no receipt
/// artifact, so this is the latest successful attempt; the workflow refuses
/// collection unless every shard job succeeded. The selected attempt is then
/// validated exactly and an invalid one is never replaced by an older attempt.
fn latest_shard_attempt(
    inputs: &Path,
    artifact_os: &str,
    shard: &str,
    max_attempt: u64,
) -> Result<(u64, PathBuf)> {
    let directory = inputs.join(shard);
    let metadata = fs::symlink_metadata(&directory)
        .with_context(|| format!("coverage shard {shard} has no downloaded artifacts"))?;
    ensure!(
        metadata.file_type().is_dir(),
        "coverage shard {shard} input is not a directory"
    );
    let entries = directory_names(&directory, shard)?;
    let flat = entries
        .iter()
        .filter(|(name, _)| name.starts_with(ATTEMPT_DIRECTORY))
        .count();
    let mut attempts = BTreeMap::new();
    if flat > 0 {
        ensure!(
            flat == entries.len(),
            "coverage shard {shard} mixes single- and multi-artifact download layouts"
        );
        ensure!(
            entries.len() == 1,
            "coverage shard {shard} has several attempts outside artifact directories"
        );
        let (name, path) = &entries[0];
        attempts.insert(evidence_attempt(shard, name)?, path.clone());
    } else {
        // Only this OS's artifacts belong here; another OS's label is refused.
        let marker = format!("-coverage-{artifact_os}-{shard}-attempt-");
        let mut prefix = None;
        for (name, path) in entries {
            let (artifact_prefix, attempt) = name
                .rsplit_once(&marker)
                .filter(|(artifact_prefix, _)| !artifact_prefix.is_empty())
                .with_context(|| format!("unexpected coverage shard {shard} artifact {name}"))?;
            let attempt = canonical_attempt(attempt).with_context(|| {
                format!("coverage shard {shard} artifact {name} has a non-canonical attempt")
            })?;
            ensure!(
                *prefix.get_or_insert_with(|| artifact_prefix.to_owned()) == artifact_prefix,
                "coverage shard {shard} artifacts use different prefixes"
            );
            let inner = directory_names(&path, shard)?;
            ensure!(
                inner.len() == 1,
                "coverage shard {shard} artifact {name} must hold one attempt directory"
            );
            let (inner_name, inner_path) = inner.into_iter().next().expect("one entry");
            ensure!(
                evidence_attempt(shard, &inner_name)? == attempt,
                "coverage shard {shard} artifact {name} holds {inner_name}"
            );
            ensure!(
                attempts.insert(attempt, inner_path).is_none(),
                "coverage shard {shard} has duplicate attempt {attempt}"
            );
        }
    }
    if let Some((&attempt, _)) = attempts.last_key_value() {
        ensure!(
            attempt <= max_attempt,
            "coverage shard {shard} attempt {attempt} is newer than run attempt {max_attempt}"
        );
    }
    attempts
        .pop_last()
        .with_context(|| format!("coverage shard {shard} has no downloaded artifacts"))
}

pub struct CollectOptions<'a> {
    pub root: &'a Path,
    pub inventory: &'a Path,
    pub inputs: &'a Path,
    pub target_dir: &'a Path,
    pub expected_source: &'a str,
    /// The current workflow run attempt; no shard may claim a later one.
    pub max_attempt: &'a str,
    /// The hosted OS label every accepted artifact name must carry.
    pub artifact_os: &'a str,
    pub llvm_cov: &'a Path,
}

/// Verify each shard's latest uploaded (successful) attempt and copy its
/// profiles. Returns the accepted attempt for every shard.
pub async fn collect_profiles(options: &CollectOptions<'_>) -> Result<Vec<(String, u64)>> {
    let CollectOptions {
        root,
        inventory: inventory_path,
        inputs,
        target_dir,
        expected_source,
        max_attempt,
        artifact_os,
        llvm_cov,
    } = options;
    let artifact_os = artifact_os_label(artifact_os)?;
    let max_attempt = canonical_attempt(max_attempt)
        .context("coverage run attempt must be a positive integer")?;
    let inventory: Inventory = read_json(inventory_path)?;
    ensure!(inventory.schema == SCHEMA, "unsupported inventory schema");
    let inventory_sha256 = digest_json(&inventory)?;
    let observed = identity(root, expected_source, llvm_cov).await?;
    collect_profiles_with_identity(
        &inventory,
        &inventory_sha256,
        inputs,
        target_dir,
        max_attempt,
        artifact_os,
        &observed,
    )
}

fn collect_profiles_with_identity(
    inventory: &Inventory,
    inventory_sha256: &str,
    inputs: &Path,
    target_dir: &Path,
    max_attempt: u64,
    artifact_os: &str,
    observed: &ReceiptIdentity,
) -> Result<Vec<(String, u64)>> {
    let artifact_os = artifact_os_label(artifact_os)?;
    ensure!(
        fs::symlink_metadata(inputs)?.file_type().is_dir(),
        "coverage inputs path is not a directory"
    );
    ensure!(
        fs::symlink_metadata(target_dir)?.file_type().is_dir(),
        "aggregate target is not a directory"
    );
    let shard_names: Vec<_> = SHARDS.iter().map(|(name, _)| *name).collect();
    exact_entries(inputs, &shard_names)?;
    let mut selected = Vec::new();
    for shard in shard_names {
        let (attempt, directory) = latest_shard_attempt(inputs, artifact_os, shard, max_attempt)?;
        selected.push((shard.to_owned(), attempt, directory));
    }
    ensure_no_profiles(target_dir)?;

    let mut seen = BTreeSet::new();
    let mut assigned = BTreeSet::new();
    let mut accepted = Vec::new();
    let mut aggregate_count = 0_usize;
    let mut aggregate_bytes = 0_u64;
    for (shard, attempt, directory) in &selected {
        ensure!(
            fs::symlink_metadata(directory)?.file_type().is_dir(),
            "shard artifact is not a directory"
        );
        exact_entries(
            directory,
            &[
                "inventory.json",
                "profiles",
                "receipt.json",
                "runner-ledger.jsonl",
                "selection.json",
            ],
        )?;
        let receipt: Receipt = read_json(&directory.join("receipt.json"))?;
        ensure!(receipt.schema == SCHEMA, "unsupported receipt schema");
        ensure!(
            receipt.shard == *shard,
            "artifact for shard {shard} carries a receipt for {}",
            receipt.shard
        );
        ensure!(
            receipt.run_attempt == attempt.to_string(),
            "shard {shard} receipt attempt differs from its artifact attempt {attempt}"
        );
        ensure!(
            seen.insert(receipt.shard.clone()),
            "duplicate shard {}",
            receipt.shard
        );
        let packages = shard_packages(&receipt.shard)?;
        ensure!(
            receipt.packages == packages,
            "wrong packages for shard {}",
            receipt.shard
        );
        for package in &receipt.packages {
            ensure!(
                assigned.insert(package.clone()),
                "package {package} is assigned twice"
            );
        }
        ensure!(receipt.source == observed.source, "shard source differs");
        ensure!(receipt.tree == observed.tree, "shard tree differs");
        ensure!(
            receipt.cargo_lock_sha256 == observed.cargo_lock_sha256,
            "shard Cargo.lock differs"
        );
        ensure!(
            receipt.rustc == observed.rustc,
            "shard Rust toolchain differs"
        );
        ensure!(
            receipt.cargo == observed.cargo,
            "shard Cargo toolchain differs"
        );
        ensure!(receipt.target == observed.target, "shard target differs");
        ensure!(
            receipt.cargo_llvm_cov == observed.cargo_llvm_cov,
            "shard cargo-llvm-cov differs"
        );
        ensure!(
            receipt.instrumentation == INSTRUMENTATION,
            "shard instrumentation contract differs"
        );
        ensure!(
            receipt.target_os == observed.target_os,
            "shard operating system differs"
        );
        ensure!(
            receipt.inventory_sha256 == inventory_sha256,
            "shard inventory differs"
        );
        let uploaded_inventory: Inventory = read_json(&directory.join("inventory.json"))?;
        ensure!(
            uploaded_inventory == *inventory,
            "uploaded shard inventory differs"
        );
        let selection: Selection = read_json(&directory.join("selection.json"))?;
        ensure!(selection.schema == SCHEMA, "unsupported selection schema");
        ensure!(
            selection.packages == receipt.packages,
            "uploaded selection packages differ"
        );
        ensure!(
            digest_json(&selection)? == receipt.selection_sha256,
            "uploaded selection differs"
        );
        ensure!(
            selection.inventory_sha256 == inventory_sha256,
            "uploaded selection names another inventory"
        );
        validate_selection(inventory, &selection.artifacts, &receipt.packages)?;
        let runner_ledger =
            read_bounded(&directory.join("runner-ledger.jsonl"), RUNNER_LEDGER_LIMIT)?;
        ensure!(
            archive::digest(&runner_ledger) == receipt.runner_ledger_sha256,
            "uploaded runner ledger differs"
        );
        validate_run_ledger(
            &directory.join("inventory.json"),
            &directory.join("selection.json"),
            &directory.join("runner-ledger.jsonl"),
        )?;

        let profile_dir = directory.join("profiles");
        ensure!(
            fs::symlink_metadata(&profile_dir)?.file_type().is_dir(),
            "shard profiles path is not a directory"
        );
        validate_profile_receipts(&receipt.profiles)?;
        aggregate_count = aggregate_count
            .checked_add(receipt.profiles.len())
            .context("aggregate profile count overflow")?;
        ensure!(
            aggregate_count <= PROFILE_COUNT_LIMIT,
            "aggregate contains too many raw profiles"
        );
        aggregate_bytes = receipt
            .profiles
            .iter()
            .try_fold(aggregate_bytes, |total, profile| {
                total
                    .checked_add(profile.bytes)
                    .context("aggregate profile size overflow")
            })?;
        ensure!(
            aggregate_bytes <= PROFILE_TOTAL_LIMIT,
            "aggregate raw profiles exceed total size limit"
        );
        let expected_names: Vec<_> = receipt
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        exact_entries(&profile_dir, &expected_names)?;
        for profile in &receipt.profiles {
            verify_profile(&profile_dir.join(&profile.name), profile)?;
        }
        accepted.push((profile_dir, receipt));
    }
    let expected_shards: BTreeSet<_> = SHARDS.iter().map(|(name, _)| (*name).to_owned()).collect();
    ensure!(seen == expected_shards, "coverage shard set differs");
    let expected_packages: BTreeSet<_> = inventory.workspace_packages.iter().cloned().collect();
    ensure!(
        assigned == expected_packages,
        "coverage package assignment differs from workspace"
    );
    for (profile_dir, receipt) in accepted {
        for profile in &receipt.profiles {
            let destination = target_dir.join(format!("{}-{}", receipt.shard, profile.name));
            copy_checked_profile(&profile_dir.join(&profile.name), &destination, profile)?;
        }
    }
    Ok(selected
        .into_iter()
        .map(|(shard, attempt, _)| (shard, attempt))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, value: &impl Serialize) {
        fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn metadata_value(root: &Path) -> serde_json::Value {
        serde_json::json!({
            "packages": [
                {
                    "id": "alpha-id", "name": "alpha",
                    "manifest_path": root.join("alpha/Cargo.toml"),
                    "targets": [{"kind": ["lib"], "name": "alpha", "test": true}]
                },
                {
                    "id": "common-id", "name": "common",
                    "manifest_path": root.join("common/Cargo.toml"),
                    "targets": [{"kind": ["lib"], "name": "common", "test": true}]
                }
            ],
            "workspace_members": ["alpha-id", "common-id"],
            "workspace_root": root
        })
    }

    fn workspace(root: &Path, target: &Path) {
        for package in ["alpha", "common"] {
            fs::create_dir_all(root.join(package).join("src")).unwrap();
            fs::write(
                root.join(package).join("Cargo.toml"),
                format!("[package]\nname = \"{package}\"\nversion = \"0.0.0\"\n"),
            )
            .unwrap();
        }
        fs::create_dir_all(target.join("debug/deps")).unwrap();
    }

    fn artifact(
        root: &Path,
        target: &Path,
        package: &str,
        name: &str,
        hash: &str,
        runnable: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": format!("{package}-id"),
            "target": {
                "kind": ["lib"], "crate_types": ["lib"], "name": name,
                "src_path": root.join(package).join("src/lib.rs"),
                "edition": "2024", "doc": true, "doctest": true, "test": true
            },
            "profile": {
                "opt_level": "0", "debuginfo": 2, "debug_assertions": true,
                "overflow_checks": true, "test": runnable
            },
            "features": [],
            "filenames": [target.join("debug/deps").join(format!("{name}-{hash}"))],
            "executable": runnable.then(|| target.join("debug/deps").join(format!("{name}-{hash}"))),
            "fresh": false
        })
    }

    fn cargo_messages(path: &Path, values: &[serde_json::Value]) {
        let mut bytes = Vec::new();
        for value in values {
            serde_json::to_writer(&mut bytes, value).unwrap();
            bytes.push(b'\n');
        }
        fs::write(path, bytes).unwrap();
    }

    /// The hosted OS label the aggregate fixture's artifacts carry.
    const FIXTURE_OS: &str = "windows-latest";

    fn receipt_identity() -> ReceiptIdentity {
        ReceiptIdentity {
            source: "source".to_owned(),
            tree: "tree".to_owned(),
            cargo_lock_sha256: "lock".to_owned(),
            rustc: "rustc".to_owned(),
            cargo: "cargo".to_owned(),
            target: "target".to_owned(),
            cargo_llvm_cov: LLVM_COV_VERSION.to_owned(),
            target_os: "windows".to_owned(),
        }
    }

    fn receipt_artifact(package: &str) -> Artifact {
        Artifact {
            package: package.to_owned(),
            package_root: format!("packages/{package}"),
            target_name: package.to_owned(),
            target_kind: vec!["lib".to_owned()],
            crate_types: vec!["lib".to_owned()],
            source: format!("packages/{package}/src/lib.rs"),
            edition: "2024".to_owned(),
            doc: true,
            doctest: true,
            target_test: true,
            profile_test: true,
            opt_level: "0".to_owned(),
            debuginfo: "2".to_owned(),
            debug_assertions: true,
            overflow_checks: true,
            features: Vec::new(),
            filenames: vec![format!("debug/deps/{package}.exe")],
            executable: Some(format!("debug/deps/{package}.exe")),
        }
    }

    #[cfg(windows)]
    #[test]
    fn coverage_breakaway_is_limited_to_the_verified_memory_library_artifact() {
        let mut memory = receipt_artifact("kuru-memory");
        assert!(needs_independent_service_lifetime(&memory));
        memory.target_kind = vec!["test".to_owned()];
        assert!(!needs_independent_service_lifetime(&memory));
        let mut runtime = receipt_artifact("kuru-runtime");
        assert!(!needs_independent_service_lifetime(&runtime));
        runtime.package = "kuru-memory".to_owned();
        assert!(!needs_independent_service_lifetime(&runtime));
    }

    struct AggregateFixture {
        _temp: TempDir,
        inputs: PathBuf,
        target: PathBuf,
        inventory: Inventory,
        inventory_sha256: String,
        identity: ReceiptIdentity,
    }

    impl AggregateFixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let inputs = temp.path().join("inputs");
            let target = temp.path().join("target");
            fs::create_dir(&inputs).unwrap();
            fs::create_dir(&target).unwrap();
            let mut workspace_packages: Vec<_> = SHARDS
                .iter()
                .flat_map(|(_, packages)| packages.iter().copied())
                .map(str::to_owned)
                .collect();
            workspace_packages.sort();
            let inventory = Inventory {
                schema: SCHEMA,
                artifacts: workspace_packages
                    .iter()
                    .map(|package| receipt_artifact(package))
                    .collect(),
                workspace_packages,
            };
            let inventory_sha256 = digest_json(&inventory).unwrap();
            let identity = receipt_identity();
            let fixture = Self {
                _temp: temp,
                inputs,
                target,
                inventory,
                inventory_sha256,
                identity,
            };
            for (shard, _) in SHARDS {
                fixture.upload(shard, 2);
            }
            fixture
        }

        fn artifact(&self, shard: &str, attempt: u64) -> PathBuf {
            self.inputs.join(shard).join(format!(
                "ci-coverage-{FIXTURE_OS}-{shard}-attempt-{attempt}"
            ))
        }

        /// The shard's evidence for one attempt in whichever layout it has.
        fn evidence(&self, shard: &str, attempt: u64) -> PathBuf {
            let flat = self.inputs.join(shard).join(attempt_directory(attempt));
            if flat.exists() {
                flat
            } else {
                self.artifact(shard, attempt)
                    .join(attempt_directory(attempt))
            }
        }

        /// Lay out a shard's artifacts as the pinned download-artifact does for
        /// a pattern download: one matching artifact is extracted directly into
        /// `<inputs>/<shard>/`, several into `<inputs>/<shard>/<artifact name>/`.
        /// Each artifact's own root holds its `attempt-<n>` directory.
        fn download_layout(&self, shard: &str) {
            let directory = self.inputs.join(shard);
            let names: Vec<_> = fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            let mut attempts = Vec::new();
            for name in names {
                if let Some(attempt) = name.strip_prefix(ATTEMPT_DIRECTORY) {
                    let attempt: u64 = attempt.parse().unwrap();
                    let artifact = self.artifact(shard, attempt);
                    fs::create_dir(&artifact).unwrap();
                    fs::rename(directory.join(&name), artifact.join(&name)).unwrap();
                    attempts.push(attempt);
                } else {
                    let (_, attempt) = name.rsplit_once("-attempt-").unwrap();
                    attempts.push(attempt.parse().unwrap());
                }
            }
            if let [attempt] = attempts[..] {
                let artifact = self.artifact(shard, attempt);
                let name = attempt_directory(attempt);
                fs::rename(artifact.join(&name), directory.join(&name)).unwrap();
                fs::remove_dir(artifact).unwrap();
            }
        }

        /// Write one checked shard artifact and lay the shard out as a pattern
        /// download would; returns the attempt's evidence directory.
        fn upload(&self, shard: &str, attempt: u64) -> PathBuf {
            let Self {
                inventory,
                inventory_sha256,
                identity,
                ..
            } = self;
            let directory = self
                .artifact(shard, attempt)
                .join(attempt_directory(attempt));
            {
                let profiles = directory.join("profiles");
                fs::create_dir_all(&profiles).unwrap();
                let packages = shard_packages(shard).unwrap();
                let selection = Selection {
                    schema: SCHEMA,
                    inventory_sha256: inventory_sha256.clone(),
                    artifacts: inventory
                        .artifacts
                        .iter()
                        .filter(|artifact| packages.binary_search(&artifact.package).is_ok())
                        .cloned()
                        .collect(),
                    packages: packages.clone(),
                };
                let bytes = format!("profile-{shard}").into_bytes();
                let profile = ProfileReceipt {
                    name: "0000.profraw".to_owned(),
                    bytes: bytes.len() as u64,
                    sha256: archive::digest(&bytes),
                };
                fs::write(profiles.join(&profile.name), bytes).unwrap();
                let mut runner_ledger = Vec::new();
                for artifact in inventory
                    .artifacts
                    .iter()
                    .filter(|artifact| is_runnable(artifact))
                {
                    serde_json::to_writer(
                        &mut runner_ledger,
                        &RunnerRecord {
                            schema: SCHEMA,
                            executable: artifact.executable.clone().unwrap(),
                            action: if packages.binary_search(&artifact.package).is_ok() {
                                "run"
                            } else {
                                "omit"
                            }
                            .to_owned(),
                            cwd: artifact.package_root.clone(),
                            args: Vec::new(),
                            success: true,
                            status_code: Some(0),
                        },
                    )
                    .unwrap();
                    runner_ledger.push(b'\n');
                }
                fs::write(directory.join("runner-ledger.jsonl"), &runner_ledger).unwrap();
                write(&directory.join("inventory.json"), &inventory);
                write(&directory.join("selection.json"), &selection);
                write(
                    &directory.join("receipt.json"),
                    &Receipt {
                        schema: SCHEMA,
                        shard: shard.to_owned(),
                        run_attempt: attempt.to_string(),
                        packages,
                        source: identity.source.clone(),
                        tree: identity.tree.clone(),
                        cargo_lock_sha256: identity.cargo_lock_sha256.clone(),
                        rustc: identity.rustc.clone(),
                        cargo: identity.cargo.clone(),
                        target: identity.target.clone(),
                        cargo_llvm_cov: identity.cargo_llvm_cov.clone(),
                        instrumentation: INSTRUMENTATION.to_owned(),
                        target_os: identity.target_os.clone(),
                        inventory_sha256: inventory_sha256.clone(),
                        selection_sha256: digest_json(&selection).unwrap(),
                        runner_ledger_sha256: archive::digest(&runner_ledger),
                        profiles: vec![profile],
                    },
                );
            }
            self.download_layout(shard);
            self.evidence(shard, attempt)
        }

        fn remove(&self, shard: &str) {
            fs::remove_dir_all(self.inputs.join(shard)).unwrap();
        }

        fn shard(&self, name: &str) -> PathBuf {
            self.evidence(name, 2)
        }

        fn collect(&self) -> Result<Vec<(String, u64)>> {
            self.collect_through(2)
        }

        fn collect_through(&self, max_attempt: u64) -> Result<Vec<(String, u64)>> {
            collect_profiles_with_identity(
                &self.inventory,
                &self.inventory_sha256,
                &self.inputs,
                &self.target,
                max_attempt,
                FIXTURE_OS,
                &self.identity,
            )
        }

        fn profile_count(&self) -> usize {
            fs::read_dir(&self.target).unwrap().count()
        }
    }

    #[test]
    fn aggregate_requires_exact_shards_receipts_and_unchanged_profiles() {
        let complete = AggregateFixture::new();
        complete.collect().unwrap();
        assert_eq!(
            fs::read_dir(&complete.target).unwrap().count(),
            SHARDS.len()
        );

        let missing = AggregateFixture::new();
        fs::remove_dir_all(missing.shard("application")).unwrap();
        let error = missing.collect().unwrap_err().to_string();
        assert!(error.contains("has no downloaded artifacts"), "{error}");
        assert_eq!(fs::read_dir(&missing.target).unwrap().count(), 0);

        let duplicate = AggregateFixture::new();
        let receipt_path = duplicate.shard("memory").join("receipt.json");
        let mut receipt: Receipt = read_json(&receipt_path).unwrap();
        receipt.shard = "application".to_owned();
        write(&receipt_path, &receipt);
        assert!(duplicate.collect().is_err());
        assert_eq!(fs::read_dir(&duplicate.target).unwrap().count(), 0);

        let wrong_package = AggregateFixture::new();
        let receipt_path = wrong_package.shard("delivery-archive").join("receipt.json");
        let mut receipt: Receipt = read_json(&receipt_path).unwrap();
        receipt.packages.pop();
        write(&receipt_path, &receipt);
        assert!(wrong_package.collect().is_err());
        assert_eq!(fs::read_dir(&wrong_package.target).unwrap().count(), 0);

        let changed_inventory = AggregateFixture::new();
        let inventory_path = changed_inventory
            .shard("connectors-core-platform")
            .join("inventory.json");
        let mut inventory: Inventory = read_json(&inventory_path).unwrap();
        inventory.artifacts.pop();
        write(&inventory_path, &inventory);
        assert!(changed_inventory.collect().is_err());
        assert_eq!(fs::read_dir(&changed_inventory.target).unwrap().count(), 0);

        let changed_feature = AggregateFixture::new();
        let inventory_path = changed_feature
            .shard("connectors-core-platform")
            .join("inventory.json");
        let mut inventory: Inventory = read_json(&inventory_path).unwrap();
        inventory.artifacts[0].features.push("changed".to_owned());
        write(&inventory_path, &inventory);
        assert!(changed_feature.collect().is_err());
        assert_eq!(fs::read_dir(&changed_feature.target).unwrap().count(), 0);

        let changed_selection = AggregateFixture::new();
        let directory = changed_selection.shard("delivery-archive");
        let selection_path = directory.join("selection.json");
        let mut selection: Selection = read_json(&selection_path).unwrap();
        selection.artifacts.pop();
        write(&selection_path, &selection);
        let receipt_path = directory.join("receipt.json");
        let mut receipt: Receipt = read_json(&receipt_path).unwrap();
        receipt.selection_sha256 = digest_json(&selection).unwrap();
        write(&receipt_path, &receipt);
        assert!(changed_selection.collect().is_err());
        assert_eq!(fs::read_dir(&changed_selection.target).unwrap().count(), 0);

        for field in ["source", "tool", "attempt"] {
            let changed_identity = AggregateFixture::new();
            let receipt_path = changed_identity.shard("application").join("receipt.json");
            let mut receipt: Receipt = read_json(&receipt_path).unwrap();
            match field {
                "source" => receipt.source.push_str("-changed"),
                "tool" => receipt.cargo_llvm_cov.push_str("-changed"),
                "attempt" => receipt.run_attempt = "3".to_owned(),
                _ => unreachable!(),
            }
            write(&receipt_path, &receipt);
            assert!(
                changed_identity.collect().is_err(),
                "accepted changed {field}"
            );
            assert_eq!(fs::read_dir(&changed_identity.target).unwrap().count(), 0);
        }

        for mutation in ["missing", "empty", "changed"] {
            let changed_profile = AggregateFixture::new();
            let profile = changed_profile
                .shard("memory")
                .join("profiles/0000.profraw");
            match mutation {
                "missing" => fs::remove_file(profile).unwrap(),
                "empty" => fs::write(profile, []).unwrap(),
                "changed" => fs::write(profile, b"different bytes").unwrap(),
                _ => unreachable!(),
            }
            assert!(
                changed_profile.collect().is_err(),
                "accepted {mutation} profile"
            );
            assert_eq!(fs::read_dir(&changed_profile.target).unwrap().count(), 0);
        }
    }

    #[test]
    fn aggregate_takes_each_shards_latest_attempt_and_fails_closed() {
        // A partial rerun uploads only the rerun shard under the new attempt.
        let rerun = AggregateFixture::new();
        rerun.upload("application", 3);
        let selected = rerun.collect_through(3).unwrap();
        let expected: Vec<_> = SHARDS
            .iter()
            .map(|(shard, _)| {
                (
                    (*shard).to_owned(),
                    if *shard == "application" { 3 } else { 2 },
                )
            })
            .collect();
        assert_eq!(selected, expected);
        assert_eq!(rerun.profile_count(), SHARDS.len());

        // A stale lower attempt is ignored once a later one exists.
        let stale = AggregateFixture::new();
        let old = stale.upload("memory", 1);
        fs::write(old.join("receipt.json"), b"not a receipt").unwrap();
        assert!(stale.collect().unwrap().contains(&("memory".to_owned(), 2)));

        // An invalid latest attempt never falls back to an older valid one.
        for mutation in ["profile", "source", "tree", "shard", "attempt"] {
            let invalid = AggregateFixture::new();
            let latest = invalid.upload("memory", 3);
            let receipt_path = latest.join("receipt.json");
            let mut receipt: Receipt = read_json(&receipt_path).unwrap();
            match mutation {
                "profile" => fs::write(latest.join("profiles/0000.profraw"), b"other").unwrap(),
                "source" => receipt.source = "other-commit".to_owned(),
                "tree" => receipt.tree = "other-tree".to_owned(),
                "shard" => receipt.shard = "application".to_owned(),
                "attempt" => receipt.run_attempt = "2".to_owned(),
                _ => unreachable!(),
            }
            if mutation != "profile" {
                fs::remove_file(&receipt_path).unwrap();
                write(&receipt_path, &receipt);
            }
            let error = invalid.collect_through(3).unwrap_err().to_string();
            let reason = match mutation {
                "profile" => "profile 0000.profraw changed",
                "source" => "shard source differs",
                "tree" => "shard tree differs",
                "shard" => "carries a receipt for application",
                "attempt" => "receipt attempt differs",
                _ => unreachable!(),
            };
            assert!(error.contains(reason), "{mutation}: {error}");
            assert_eq!(invalid.profile_count(), 0);
        }

        // No shard may claim an attempt later than the current run attempt,
        // in either download layout.
        let future = AggregateFixture::new();
        future.upload("application", 3);
        let error = future.collect_through(2).unwrap_err().to_string();
        assert!(error.contains("newer than run attempt 2"), "{error}");
        assert_eq!(future.profile_count(), 0);
        let flat_future = AggregateFixture::new();
        let error = flat_future.collect_through(1).unwrap_err().to_string();
        assert!(error.contains("newer than run attempt 1"), "{error}");
        assert_eq!(flat_future.profile_count(), 0);

        // Missing shards and unknown shard directories are refused.
        let missing = AggregateFixture::new();
        missing.remove("connectors-core-platform");
        let error = missing.collect().unwrap_err().to_string();
        assert!(error.contains("unexpected artifact entries"), "{error}");
        assert_eq!(missing.profile_count(), 0);
        let empty = AggregateFixture::new();
        fs::remove_dir_all(empty.shard("connectors-core-platform")).unwrap();
        let error = empty.collect().unwrap_err().to_string();
        assert!(error.contains("has no downloaded artifacts"), "{error}");
        assert_eq!(empty.profile_count(), 0);
        let unknown = AggregateFixture::new();
        fs::create_dir(unknown.inputs.join("unknown")).unwrap();
        let error = unknown.collect().unwrap_err().to_string();
        assert!(error.contains("unexpected artifact entries"), "{error}");
        assert!(error.contains("unknown"), "{error}");
        assert_eq!(unknown.profile_count(), 0);

        // Non-canonical or foreign artifact names beside real artifacts.
        for (name, reason) in [
            (
                "ci-coverage-windows-latest-application-attempt-03",
                "non-canonical attempt",
            ),
            (
                "ci-coverage-windows-latest-application-attempt-0",
                "non-canonical attempt",
            ),
            (
                "ci-coverage-windows-latest-application-attempt-x",
                "non-canonical attempt",
            ),
            (
                "ci-coverage-windows-latest-application-diagnostics-attempt-1",
                "unexpected coverage shard",
            ),
            (
                "other-coverage-windows-latest-application-attempt-1",
                "different prefixes",
            ),
            (
                "-coverage-windows-latest-application-attempt-1",
                "unexpected coverage shard",
            ),
            // Another OS's receipt for the same shard is never accepted here.
            (
                "ci-coverage-ubuntu-latest-application-attempt-1",
                "unexpected coverage shard",
            ),
            (
                "ci-coverage-windows-application-attempt-1",
                "unexpected coverage shard",
            ),
        ] {
            let foreign = AggregateFixture::new();
            foreign.upload("application", 3);
            fs::create_dir(foreign.inputs.join("application").join(name)).unwrap();
            let error = foreign.collect_through(3).unwrap_err().to_string();
            assert!(error.contains(reason), "{name}: {error}");
            assert_eq!(foreign.profile_count(), 0);
        }

        // Layout refusals: a nested artifact must hold exactly its own attempt,
        // a flat download holds one canonical attempt, and layouts never mix.
        let mismatched = AggregateFixture::new();
        let latest = mismatched.upload("application", 3);
        fs::rename(&latest, latest.with_file_name("attempt-4")).unwrap();
        let error = mismatched.collect_through(4).unwrap_err().to_string();
        assert!(
            error.contains("attempt-3 holds attempt-4"),
            "mismatched: {error}"
        );
        let extra = AggregateFixture::new();
        let latest = extra.upload("application", 3);
        fs::create_dir(latest.with_file_name("attempt-1")).unwrap();
        let error = extra.collect_through(3).unwrap_err().to_string();
        assert!(error.contains("must hold one attempt directory"), "{error}");
        let mixed = AggregateFixture::new();
        fs::create_dir(mixed.artifact("application", 1)).unwrap();
        let error = mixed.collect().unwrap_err().to_string();
        assert!(
            error.contains("mixes single- and multi-artifact"),
            "{error}"
        );
        let several_flat = AggregateFixture::new();
        fs::create_dir(several_flat.inputs.join("application/attempt-1")).unwrap();
        let error = several_flat.collect().unwrap_err().to_string();
        assert!(
            error.contains("several attempts outside artifact directories"),
            "{error}"
        );
        let padded = AggregateFixture::new();
        let flat = padded.shard("application");
        fs::rename(&flat, flat.with_file_name("attempt-02")).unwrap();
        let error = padded.collect().unwrap_err().to_string();
        assert!(
            error.contains("not a canonical attempt directory"),
            "{error}"
        );
        let loose = AggregateFixture::new();
        fs::write(loose.inputs.join("application/attempt-1"), b"").unwrap();
        let error = loose.collect().unwrap_err().to_string();
        assert!(error.contains("attempt-1 is not a directory"), "{error}");
        for fixture in [&mismatched, &extra, &mixed, &several_flat, &padded, &loose] {
            assert_eq!(fixture.profile_count(), 0);
        }
    }

    #[test]
    fn aggregate_accepts_the_single_artifact_download_layout() {
        // Every ordinary first attempt downloads one artifact per shard, which
        // download-artifact extracts without an artifact-name directory.
        let first = AggregateFixture::new();
        for (shard, _) in SHARDS {
            let entries: Vec<_> = fs::read_dir(first.inputs.join(shard))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            assert_eq!(entries, ["attempt-2"]);
        }
        let selected = first.collect().unwrap();
        assert!(selected.iter().all(|(_, attempt)| *attempt == 2));
        assert_eq!(first.profile_count(), SHARDS.len());

        // Several attempts of one shard use the per-artifact layout while the
        // other shards stay flat.
        let rerun = AggregateFixture::new();
        rerun.upload("memory", 3);
        assert!(rerun.artifact("memory", 2).join("attempt-2").is_dir());
        assert!(rerun.artifact("memory", 3).join("attempt-3").is_dir());
        assert!(rerun.inputs.join("application/attempt-2").is_dir());
        assert!(
            rerun
                .collect_through(3)
                .unwrap()
                .contains(&("memory".to_owned(), 3))
        );
    }

    #[test]
    fn shards_cover_each_workspace_package_exactly_once() {
        let listed: Vec<_> = SHARDS
            .iter()
            .flat_map(|(_, packages)| packages.iter().copied())
            .collect();
        let unique = workspace_packages();
        assert_eq!(listed.len(), unique.len(), "a package is in two shards");
        assert_eq!(
            unique.into_iter().collect::<Vec<_>>(),
            [
                "kuru",
                "kuru-archive",
                "kuru-connectors",
                "kuru-core",
                "kuru-delivery",
                "kuru-memory",
                "kuru-platform",
                "kuru-runtime",
            ]
        );
        let names: BTreeSet<_> = SHARDS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), SHARDS.len(), "shard names are unique");
        for (name, packages) in SHARDS {
            assert!(!packages.is_empty(), "{name} has no packages");
            assert!(artifact_os_label(name).is_ok(), "{name} is not a label");
        }
    }

    #[test]
    fn artifact_os_labels_are_lowercase_runner_names() {
        for label in [
            "ubuntu-latest",
            "macos-latest",
            "windows-2025",
            "windows-latest",
        ] {
            assert_eq!(artifact_os_label(label).unwrap(), label);
        }
        for label in [
            "",
            "Windows-latest",
            "ubuntu_latest",
            "macos latest",
            "../x",
            "a/b",
        ] {
            let error = artifact_os_label(label).unwrap_err().to_string();
            assert!(error.contains("[a-z0-9-]+"), "{label}: {error}");
        }
        let fixture = AggregateFixture::new();
        let error = collect_profiles_with_identity(
            &fixture.inventory,
            &fixture.inventory_sha256,
            &fixture.inputs,
            &fixture.target,
            2,
            "Windows",
            &fixture.identity,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("[a-z0-9-]+"), "{error}");
        assert_eq!(fixture.profile_count(), 0);

        // Artifacts for another OS label never satisfy this OS's collection.
        let rerun = AggregateFixture::new();
        rerun.upload("memory", 3);
        let error = collect_profiles_with_identity(
            &rerun.inventory,
            &rerun.inventory_sha256,
            &rerun.inputs,
            &rerun.target,
            3,
            "ubuntu-latest",
            &rerun.identity,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("unexpected coverage shard memory"),
            "{error}"
        );
        assert_eq!(rerun.profile_count(), 0);
    }

    #[tokio::test]
    async fn receipts_require_a_canonical_run_attempt() {
        let temp = TempDir::new().unwrap();
        let path = temp.path();
        for attempt in ["", "0", "03", "x"] {
            let error = write_receipt(&ReceiptOptions {
                root: path,
                inventory: path,
                selection: path,
                ledger: path,
                profiles: path,
                shard: "application",
                run_attempt: attempt,
                expected_source: "HEAD",
                llvm_cov: path,
                output: &path.join("output"),
            })
            .await
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("positive integer without leading zeros"),
                "{attempt}: {error}"
            );
        }
        assert!(!path.join("output").exists());
    }

    #[test]
    fn shard_deadline_keeps_the_evidence_reserve_inside_the_job_limit() {
        assert_eq!(
            shard_deadline(1_000, 90).unwrap(),
            1_000 + 90 * 60 - EVIDENCE_RESERVE.as_secs()
        );
        assert!(shard_deadline(1_000, EVIDENCE_RESERVE.as_secs() / 60).is_err());
        assert!(shard_deadline(1_000, 0).is_err());
        assert!(shard_deadline(u64::MAX, 90).is_err());
        assert!(shard_deadline(1_000, u64::MAX).is_err());
    }

    #[test]
    fn libtest_progress_names_unfinished_tests_and_recent_results() {
        let mut progress = LibtestProgress::default();
        progress
            .observe(b"\nrunning 4 tests\r\ntest slow::a has been running for over 60 seconds\n");
        progress.observe(b"test slow::b has been running for over 60 seconds\ntest fast ... o");
        progress.observe(
            b"k\ntest slow::a ... ok\ntest slow::b has been running for over 60 seconds\n",
        );
        progress.observe(b"test failing ... FAILED\nnot a test line\ntest slow::c ... ");
        assert_eq!(progress.unfinished(), ["slow::b", "slow::c"]);
        assert_eq!(
            progress.recent,
            [
                "test fast ... ok",
                "test slow::a ... ok",
                "test failing ... FAILED"
            ]
        );
        assert_eq!(progress.completed, 3);

        let mut bounded = LibtestProgress::default();
        for index in 0..RECENT_RESULT_LIMIT + 5 {
            bounded.observe(format!("test t{index} ... ok\n").as_bytes());
        }
        assert_eq!(bounded.recent.len(), RECENT_RESULT_LIMIT);
        assert_eq!(bounded.recent.front().unwrap(), "test t5 ... ok");
        assert_eq!(bounded.completed, RECENT_RESULT_LIMIT + 5);

        // An overlong line is dropped whole instead of growing without bound.
        let mut overlong = LibtestProgress::default();
        overlong.observe(b"test ");
        overlong.observe(&vec![b'x'; PENDING_LINE_LIMIT + 1]);
        overlong.observe(b" ... ");
        assert!(overlong.unfinished().is_empty());
        overlong.observe(b"\ntest after has been running for over 60 seconds\n");
        assert_eq!(overlong.unfinished(), ["after"]);
        assert_eq!(overlong.completed, 0);
    }

    struct FakeProcess {
        exit_after: Option<Duration>,
        terminated: bool,
        terminate_result: bool,
    }

    impl TestProcess for FakeProcess {
        async fn wait(&mut self, timeout: Duration) -> std::io::Result<ExitStatus> {
            #[cfg(unix)]
            use std::os::unix::process::ExitStatusExt;
            #[cfg(windows)]
            use std::os::windows::process::ExitStatusExt;

            if self.terminated {
                return Ok(ExitStatus::from_raw(1));
            }
            match self.exit_after {
                Some(after) if after <= timeout => {
                    tokio::time::sleep(after).await;
                    Ok(ExitStatus::from_raw(0))
                }
                _ => {
                    tokio::time::sleep(timeout).await;
                    Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "native process tree did not become quiescent",
                    ))
                }
            }
        }
        fn sample(&self) -> Option<String> {
            Some("kernel_time=0ns".to_owned())
        }
        fn terminate(&mut self) -> std::io::Result<()> {
            self.terminated = self.terminate_result;
            if self.terminate_result {
                Ok(())
            } else {
                Err(std::io::Error::other("termination refused"))
            }
        }
    }

    // Supervision tests run on paused Tokio time: the fake process sleeps for
    // exactly the requested wait, which the runtime advances once every other
    // task is idle, so outcomes do not depend on host scheduling.
    const FAKE_DEADLINE: Duration = Duration::from_secs(600);
    const FAKE_CLEANUP: Duration = Duration::from_secs(30);

    /// A job-log destination that refuses every write.
    struct BrokenRelay;

    impl tokio::io::AsyncWrite for BrokenRelay {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Err(std::io::Error::other("job log closed")))
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn relay_keeps_draining_after_log_or_relay_errors() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // On paused time this bound elapses only if the relay stops draining
        // and every task is parked, turning that deadlock into a failure.
        const DRAINED: Duration = Duration::from_secs(1);
        let output = b"running 2 tests\ntest one ... ok\ntest two ... FAILED\n";

        // An unavailable log leaves every byte relayed and progress observed.
        let temp = TempDir::new().unwrap();
        let missing_log = temp.path().join("absent/test.stdout.log");
        let (mut writer, reader) = tokio::io::duplex(8);
        let (relay, mut relayed) = tokio::io::duplex(1024);
        let mut progress = LibtestProgress::default();
        let (outcome, ()) = tokio::time::timeout(DRAINED, async {
            tokio::join!(
                relay_output(reader, relay, &missing_log, &mut progress),
                async {
                    // The pipe is smaller than the output, so every chunk after
                    // the first is written only because the relay kept draining.
                    writer.write_all(output).await.unwrap();
                    drop(writer);
                }
            )
        })
        .await
        .expect("relay stopped draining after the log error");
        assert!(outcome.starts_with("log create "), "{outcome}");
        assert!(outcome.contains("test.stdout.log failed"), "{outcome}");
        let mut copied = Vec::new();
        relayed.read_to_end(&mut copied).await.unwrap();
        assert_eq!(copied, output);
        assert_eq!(progress.completed, 2);
        assert!(!missing_log.exists());

        // A failed job-log relay still drains the pipe and keeps the log.
        let log = temp.path().join("relay.stdout.log");
        let (mut writer, reader) = tokio::io::duplex(8);
        let mut progress = LibtestProgress::default();
        let (outcome, ()) = tokio::time::timeout(DRAINED, async {
            tokio::join!(
                relay_output(reader, BrokenRelay, &log, &mut progress),
                async {
                    writer.write_all(output).await.unwrap();
                    drop(writer);
                }
            )
        })
        .await
        .expect("relay stopped draining after the relay error");
        assert_eq!(outcome, "job log relay failed: job log closed");
        assert_eq!(fs::read(&log).unwrap(), output);
        assert_eq!(progress.completed, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn supervision_relays_completed_output_and_keeps_a_log() {
        use tokio::io::AsyncWriteExt;

        let temp = TempDir::new().unwrap();
        let log = temp.path().join("test.stdout.log");
        let (mut writer, reader) = tokio::io::duplex(1024);
        writer
            .write_all(b"running 1 test\ntest one ... ok\n")
            .await
            .unwrap();
        drop(writer);
        let mut process = FakeProcess {
            exit_after: Some(Duration::ZERO),
            terminated: false,
            terminate_result: true,
        };
        let (relay, mut relayed) = tokio::io::duplex(1024);
        let supervision = supervise(
            &mut process,
            reader,
            relay,
            log.clone(),
            FAKE_DEADLINE,
            FAKE_CLEANUP,
        )
        .await
        .unwrap();
        assert!(matches!(supervision, Supervision::Exited(status) if status.success()));
        assert!(!process.terminated);
        let mut copied = String::new();
        tokio::io::AsyncReadExt::read_to_string(&mut relayed, &mut copied)
            .await
            .unwrap();
        assert_eq!(copied, "running 1 test\ntest one ... ok\n");
        assert_eq!(fs::read_to_string(&log).unwrap(), copied);
    }

    #[tokio::test(start_paused = true)]
    async fn supervision_terminates_a_stalled_tree_at_the_deadline_with_evidence() {
        use tokio::io::AsyncWriteExt;

        let temp = TempDir::new().unwrap();
        let log = temp.path().join("stalled.stdout.log");
        // The writer stays open, as when a process outside the owned tree
        // retains the pipe; the relay drain must still be bounded.
        let (mut writer, reader) = tokio::io::duplex(4096);
        writer
            .write_all(
                b"running 3 tests\ntest done ... ok\ntest stuck has been running for over 60 seconds\n",
            )
            .await
            .unwrap();
        let mut process = FakeProcess {
            exit_after: None,
            terminated: false,
            terminate_result: true,
        };
        let started = tokio::time::Instant::now();
        let supervision = supervise(
            &mut process,
            reader,
            tokio::io::sink(),
            log.clone(),
            FAKE_DEADLINE,
            FAKE_CLEANUP,
        )
        .await
        .unwrap();
        // The fake wait consumed the whole deadline and then the bounded drain
        // of the still-open pipe; paused time makes both exact.
        assert_eq!(started.elapsed(), FAKE_DEADLINE + FAKE_CLEANUP);
        assert!(process.terminated);
        let Supervision::Stalled(evidence) = supervision else {
            panic!("stalled tree exited");
        };
        assert_eq!(evidence.progress.unfinished(), ["stuck"]);
        assert_eq!(evidence.progress.recent, ["test done ... ok"]);
        assert_eq!(evidence.sample.as_deref(), Some("kernel_time=0ns"));
        assert_eq!(evidence.termination, "Ok(())");
        assert!(evidence.cleanup.starts_with("Ok("), "{}", evidence.cleanup);
        assert!(
            evidence.output.starts_with("stopped after"),
            "{}",
            evidence.output
        );
        assert!(
            fs::read_to_string(&log)
                .unwrap()
                .contains("test stuck has been running")
        );

        let artifact = receipt_artifact("kuru-runtime");
        let report = stall_report(
            &artifact,
            "debug/deps/kuru_runtime.exe",
            1_000,
            evidence,
            &log,
        )
        .unwrap();
        assert_eq!(report.unfinished_tests, ["stuck"]);
        assert_eq!(report.package, "kuru-runtime");
        assert_eq!(report.stdout_log, "stalled.stdout.log");
        drop(writer);

        // A tree that cannot be terminated still reports its failed cleanup.
        let (_writer, reader) = tokio::io::duplex(64);
        let mut refusing = FakeProcess {
            exit_after: None,
            terminated: false,
            terminate_result: false,
        };
        let Supervision::Stalled(evidence) = supervise(
            &mut refusing,
            reader,
            tokio::io::sink(),
            temp.path().join("refusing.stdout.log"),
            FAKE_DEADLINE,
            FAKE_CLEANUP,
        )
        .await
        .unwrap() else {
            panic!("refusing tree exited");
        };
        assert!(evidence.termination.contains("termination refused"));
        assert!(evidence.cleanup.starts_with("Err("), "{}", evidence.cleanup);
    }

    #[cfg(unix)]
    fn group_command(script: &str) -> std::process::Command {
        // The environment, including LLVM_PROFILE_FILE, is inherited unchanged.
        let mut command = std::process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(script)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        command
    }

    #[cfg(unix)]
    const GROUP_BOUND: Duration = Duration::from_secs(20);

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_group_exit_preserves_status_and_kills_group_before_reap() {
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("exit.stdout.log");
        let (relay, mut relayed) = tokio::io::duplex(4096);
        let started = std::time::Instant::now();
        // The background sleep stays in the root's group and holds its stdout.
        // Only the post-exit group signal lets the relay reach end of file.
        let supervision = supervise_group(
            group_command("sleep 300 & printf 'test one ... ok\\n'; exit 3"),
            relay,
            log.clone(),
            GROUP_BOUND,
            GROUP_BOUND,
        )
        .await
        .unwrap();
        let Supervision::Exited(status) = supervision else {
            panic!("an exiting root was reported as stalled");
        };
        assert_eq!(status.code(), Some(3), "{status:?}");
        assert!(
            started.elapsed() < GROUP_BOUND,
            "the relay waited for the group member instead of signalling it"
        );
        let mut copied = String::new();
        tokio::io::AsyncReadExt::read_to_string(&mut relayed, &mut copied)
            .await
            .unwrap();
        assert_eq!(copied, "test one ... ok\n");
        assert_eq!(fs::read_to_string(&log).unwrap(), copied);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_group_observes_presence_only_after_reaping_its_root() {
        let mut process =
            GroupProcess::spawn(group_command("sleep 300 & wait"), GROUP_BOUND).unwrap();
        let _output = process.take_stdout().unwrap();
        let error = process.wait(Duration::from_millis(100)).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(process.sample().as_deref(), Some("root_state=Running"));
        assert_eq!(process.presence_after_reap(), None);
        process.terminate().unwrap();
        // Before the reap the group is still unobserved.
        assert_eq!(process.presence_after_reap(), None);
        let status = process.wait(GROUP_BOUND).await.unwrap();
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(9),
            "{status:?}"
        );
        assert_eq!(process.presence_after_reap().as_deref(), Some("Absent"));
        // The consumed transition and cached reap are stable.
        process.terminate().unwrap();
        assert_eq!(process.wait(GROUP_BOUND).await.unwrap(), status);
        assert!(process.sample().unwrap().starts_with("root_state=Reaped("));

        // An ordinary exit without any remaining member settles the same way.
        let mut quick = GroupProcess::spawn(group_command("exit 0"), GROUP_BOUND).unwrap();
        let _output = quick.take_stdout().unwrap();
        assert!(quick.wait(GROUP_BOUND).await.unwrap().success());
        assert_eq!(quick.presence_after_reap().as_deref(), Some("Absent"));

        // A missing stdout pipe is refused after the group is cleaned up.
        let mut unpiped = group_command("sleep 300");
        unpiped.stdout(std::process::Stdio::null());
        let temp = TempDir::new().unwrap();
        let error = supervise_group(
            unpiped,
            tokio::io::sink(),
            temp.path().join("unpiped.stdout.log"),
            GROUP_BOUND,
            GROUP_BOUND,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("has no stdout pipe"), "{error}");
        assert!(error.contains("cleanup=Ok("), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_group_stall_is_terminated_reaped_and_reported() {
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("stall.stdout.log");
        let script = "printf 'running 2 tests\\ntest done ... ok\\n'; \
            printf 'test stuck has been running for over 60 seconds\\n'; \
            sleep 300 & wait";
        let supervision = supervise_group(
            group_command(script),
            tokio::io::sink(),
            log.clone(),
            Duration::from_millis(750),
            GROUP_BOUND,
        )
        .await
        .unwrap();
        let Supervision::Stalled(evidence) = supervision else {
            panic!("a stalled group was reported as exited");
        };
        assert_eq!(evidence.progress.unfinished(), ["stuck"]);
        assert_eq!(evidence.progress.recent, ["test done ... ok"]);
        assert_eq!(evidence.sample.as_deref(), Some("root_state=Running"));
        assert_eq!(evidence.termination, "Ok(())");
        assert!(evidence.cleanup.starts_with("Ok("), "{}", evidence.cleanup);
        assert_eq!(evidence.presence_after_reap.as_deref(), Some("Absent"));
        // Every group member is gone, so the relay reached end of file.
        assert_eq!(evidence.output, "complete");

        let report = stall_report(
            &receipt_artifact("kuru-core"),
            "debug/deps/kuru_core-0a1b",
            1_000,
            evidence,
            &log,
        )
        .unwrap();
        let path = temp.path().join("stall.json");
        write_json(&path, &report).unwrap();
        let written: serde_json::Value = read_json(&path).unwrap();
        assert_eq!(written["presence_after_reap"], "Absent");
        assert_eq!(written["unfinished_tests"], serde_json::json!(["stuck"]));
        assert_eq!(read_json::<StallReport>(&path).unwrap(), report);
    }

    #[test]
    fn diagnostic_names_are_single_path_components() {
        assert_eq!(
            diagnostic_name("debug/deps/kuru_runtime-0a1b.exe"),
            "debug_deps_kuru_runtime-0a1b.exe"
        );
        assert_eq!(diagnostic_name("..\\x y"), ".._x_y");
    }

    #[test]
    fn selected_manifest_is_the_exact_full_inventory_subset() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let target = temp.path().join("target");
        workspace(&root, &target);
        let metadata_path = temp.path().join("metadata.json");
        write(&metadata_path, &metadata_value(&root));
        let full_messages = temp.path().join("full.json");
        cargo_messages(
            &full_messages,
            &[
                artifact(&root, &target, "alpha", "alpha", "normal", false),
                artifact(&root, &target, "alpha", "alpha", "test", true),
                artifact(&root, &target, "common", "common", "normal", false),
                artifact(&root, &target, "common", "common", "test", true),
            ],
        );
        let inventory_path = temp.path().join("inventory.json");
        write_inventory(&metadata_path, &full_messages, &target, &inventory_path).unwrap();
        let selection_path = temp.path().join("selection.json");
        write_selection(&inventory_path, &["alpha".to_owned()], &selection_path).unwrap();
        let selection: Selection = read_json(&selection_path).unwrap();
        assert_eq!(selection.artifacts.len(), 2);
        assert!(
            selection
                .artifacts
                .iter()
                .all(|artifact| artifact.package == "alpha")
        );
        assert!(selection.artifacts.iter().any(is_runnable));
    }

    #[test]
    fn selected_manifest_rejects_runnable_artifact_outside_shard() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let target = temp.path().join("target");
        workspace(&root, &target);
        let metadata_path = temp.path().join("metadata.json");
        write(&metadata_path, &metadata_value(&root));
        let messages = temp.path().join("messages.json");
        cargo_messages(
            &messages,
            &[
                artifact(&root, &target, "alpha", "alpha", "normal", false),
                artifact(&root, &target, "alpha", "alpha", "test", true),
                artifact(&root, &target, "common", "common", "normal", false),
                artifact(&root, &target, "common", "common", "test", true),
            ],
        );
        let inventory_path = temp.path().join("inventory.json");
        write_inventory(&metadata_path, &messages, &target, &inventory_path).unwrap();
        let selection_path = temp.path().join("selection.json");
        write_selection(&inventory_path, &["alpha".to_owned()], &selection_path).unwrap();
        let mut selection: Selection = read_json(&selection_path).unwrap();
        let inventory: Inventory = read_json(&inventory_path).unwrap();
        selection.artifacts.push(
            inventory
                .artifacts
                .into_iter()
                .find(|artifact| artifact.package == "common" && is_runnable(artifact))
                .unwrap(),
        );
        fs::write(&selection_path, serde_json::to_vec(&selection).unwrap()).unwrap();
        let error = runnable_artifacts(&inventory_path, &selection_path).unwrap_err();
        assert!(error.to_string().contains("artifact outside its shard"));
    }

    #[test]
    fn cargo_runner_config_and_ledger_bind_the_full_test_inventory() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let target = temp.path().join("target");
        workspace(&root, &target);
        let inventory = Inventory {
            schema: SCHEMA,
            workspace_packages: vec!["alpha".to_owned(), "common".to_owned()],
            artifacts: vec![receipt_artifact("alpha"), receipt_artifact("common")],
        };
        let inventory_path = temp.path().join("inventory.json");
        write(&inventory_path, &inventory);
        let selection_path = temp.path().join("selection.json");
        write_selection(&inventory_path, &["alpha".to_owned()], &selection_path).unwrap();
        let helper = temp.path().join("delivery helper.exe");
        fs::write(&helper, []).unwrap();
        let ledger = temp.path().join("ledger.jsonl");
        let diagnostics = temp.path().join("diagnostics");
        fs::create_dir(&diagnostics).unwrap();
        let config = temp.path().join("runner.toml");
        let job_started = unix_now().unwrap() - 60;
        let options = |job_started, job_minutes, output| RunnerConfigOptions {
            root: &root,
            host: "x86_64-pc-windows-msvc",
            helper: &helper,
            inventory: &inventory_path,
            selection: &selection_path,
            target_dir: &target,
            ledger: &ledger,
            diagnostics: &diagnostics,
            job_started,
            job_minutes,
            output,
        };
        // A job whose remaining budget is already inside the evidence reserve,
        // or whose start is in the future, cannot configure a runner.
        let expired = temp.path().join("expired.toml");
        assert!(write_runner_config(&options(job_started - 90 * 60, 90, &expired)).is_err());
        let future = temp.path().join("future.toml");
        assert!(write_runner_config(&options(job_started + 3600, 90, &future)).is_err());
        assert!(!expired.exists() && !future.exists());
        write_runner_config(&options(job_started, 90, &config)).unwrap();
        let config: toml::Value = toml::from_str(&fs::read_to_string(config).unwrap()).unwrap();
        let runner = config["target"]["x86_64-pc-windows-msvc"]["runner"]
            .as_array()
            .unwrap();
        assert_eq!(runner[0].as_str(), helper.to_str());
        assert_eq!(runner.last().unwrap().as_str(), Some("--"));
        let argument = |flag: &str| {
            let index = runner
                .iter()
                .position(|value| value.as_str() == Some(flag))
                .unwrap();
            runner[index + 1].as_str().unwrap().to_owned()
        };
        assert_eq!(argument("--diagnostics"), diagnostics.to_str().unwrap());
        assert_eq!(
            argument("--deadline"),
            (job_started + 90 * 60 - EVIDENCE_RESERVE.as_secs()).to_string()
        );

        let records = [
            RunnerRecord {
                schema: SCHEMA,
                executable: "debug/deps/alpha.exe".to_owned(),
                action: "run".to_owned(),
                cwd: "packages/alpha".to_owned(),
                args: Vec::new(),
                success: true,
                status_code: Some(0),
            },
            RunnerRecord {
                schema: SCHEMA,
                executable: "debug/deps/common.exe".to_owned(),
                action: "omit".to_owned(),
                cwd: "packages/common".to_owned(),
                args: Vec::new(),
                success: true,
                status_code: Some(0),
            },
        ];
        let mut bytes = Vec::new();
        for record in &records {
            serde_json::to_writer(&mut bytes, record).unwrap();
            bytes.push(b'\n');
        }
        fs::write(&ledger, &bytes).unwrap();
        validate_run_ledger(&inventory_path, &selection_path, &ledger).unwrap();

        bytes.extend_from_slice(&bytes.clone());
        fs::write(&ledger, bytes).unwrap();
        assert!(validate_run_ledger(&inventory_path, &selection_path, &ledger).is_err());
    }

    #[test]
    fn profile_receipts_must_be_nonempty_unique_normal_files() {
        assert!(validate_profile_receipts(&[]).is_err());
        let receipt = ProfileReceipt {
            name: "0000.profraw".to_owned(),
            bytes: 1,
            sha256: "a".repeat(64),
        };
        assert!(validate_profile_receipts(std::slice::from_ref(&receipt)).is_ok());
        assert!(validate_profile_receipts(&[receipt.clone(), receipt]).is_err());
        assert!(
            validate_profile_receipts(&[ProfileReceipt {
                name: "../escape.profraw".to_owned(),
                bytes: 1,
                sha256: "a".repeat(64),
            }])
            .is_err()
        );
    }

    #[test]
    fn empty_profiles_are_omitted_only_after_candidate_bounds_and_type_checks() {
        let mixed = TempDir::new().unwrap();
        fs::write(mixed.path().join("empty.profraw"), []).unwrap();
        fs::write(mixed.path().join("valid.profraw"), b"valid profile").unwrap();
        let sources = profile_sources(mixed.path()).unwrap();
        assert_eq!(sources, [mixed.path().join("valid.profraw")]);
        let copied = mixed.path().join("copied.profraw");
        let receipt = hash_copy(&sources[0], &copied).unwrap();
        assert_eq!(receipt.bytes, 13);
        assert_eq!(fs::read(copied).unwrap(), b"valid profile");

        let empty = TempDir::new().unwrap();
        fs::write(empty.path().join("empty.profraw"), []).unwrap();
        assert!(
            profile_sources(empty.path())
                .unwrap_err()
                .to_string()
                .contains("no nonempty raw profiles")
        );

        let excess = TempDir::new().unwrap();
        for index in 0..=PROFILE_COUNT_LIMIT {
            fs::write(excess.path().join(format!("{index:04}.profraw")), []).unwrap();
        }
        assert!(
            profile_sources(excess.path())
                .unwrap_err()
                .to_string()
                .contains("too many profiles")
        );

        let oversized = TempDir::new().unwrap();
        File::create(oversized.path().join("oversized.profraw"))
            .unwrap()
            .set_len(PROFILE_LIMIT + 1)
            .unwrap();
        assert!(
            profile_sources(oversized.path())
                .unwrap_err()
                .to_string()
                .contains("is too large")
        );

        let nonregular = TempDir::new().unwrap();
        fs::create_dir(nonregular.path().join("directory.profraw")).unwrap();
        assert!(
            profile_sources(nonregular.path())
                .unwrap_err()
                .to_string()
                .contains("is not a regular file")
        );

        let malformed = TempDir::new().unwrap();
        let malformed_profile = malformed.path().join("malformed.profraw");
        fs::write(&malformed_profile, b"not-a-profile").unwrap();
        assert_eq!(
            profile_sources(malformed.path()).unwrap(),
            [malformed_profile]
        );
    }

    #[cfg(unix)]
    #[test]
    fn profile_symlinks_are_rejected_before_empty_filtering() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let target = temp.path().join("empty");
        fs::write(&target, []).unwrap();
        symlink(&target, temp.path().join("linked.profraw")).unwrap();
        assert!(
            profile_sources(temp.path())
                .unwrap_err()
                .to_string()
                .contains("is not a regular file")
        );
    }

    #[test]
    fn changed_profile_bytes_are_rejected() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("0000.profraw");
        fs::write(&source, b"changed profile").unwrap();
        let expected = ProfileReceipt {
            name: "0000.profraw".to_owned(),
            bytes: 15,
            sha256: "0".repeat(64),
        };
        let error = copy_checked_profile(&source, &temp.path().join("copy.profraw"), &expected)
            .unwrap_err();
        assert!(error.to_string().contains("profile 0000.profraw changed"));
    }

    #[tokio::test]
    async fn modified_tracked_source_cannot_claim_head_identity() {
        async fn git(root: &Path, args: &[&str]) -> std::process::Output {
            let mut command = crate::command::rooted(root, "git");
            command.args(["-c", "commit.gpgSign=false"]).args(args);
            crate::command::bounded_output(
                &mut command,
                std::time::Duration::from_secs(30),
                64 * 1024,
            )
            .await
            .unwrap()
        }

        let temp = TempDir::new().unwrap();
        let root = temp.path();
        for args in [
            &["init", "-b", "main"][..],
            &["config", "user.name", "Coverage fixture"][..],
            &["config", "user.email", "fixture@example.invalid"][..],
        ] {
            assert!(git(root, args).await.status.success());
        }
        fs::write(root.join("Cargo.lock"), "version = 4\n").unwrap();
        fs::write(root.join("source.rs"), "const VALUE: u8 = 1;\n").unwrap();
        assert!(git(root, &["add", "."]).await.status.success());
        assert!(
            git(root, &["commit", "-m", "fixture"])
                .await
                .status
                .success()
        );
        let head = String::from_utf8(git(root, &["rev-parse", "HEAD"]).await.stdout).unwrap();
        fs::write(root.join("source.rs"), "const VALUE: u8 = 2;\n").unwrap();
        let error = identity(root, head.trim(), Path::new("missing-llvm-cov"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("modified tracked files"));
    }
}
