//! Fail-closed manifests for partitioned native test and coverage evidence.

#[cfg(test)]
mod fixture;
pub mod lcov;
pub mod ledger;
pub mod lines;
pub mod merge;
pub mod orchestrate;
pub mod partition;
pub mod plan;
pub mod seed;
pub mod spawns;

use crate::{archive, command};
use anyhow::{Context, Result, bail, ensure};
use partition::PartitionScheme;
use plan::{InvocationRecord, RunnerRecord};
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

/// Schema of every inventory, plan, ledger and receipt. Schema 1 (package
/// shards with uploaded raw profiles) is refused by name.
const SCHEMA: u32 = 2;
const LLVM_COV_VERSION: &str = "cargo-llvm-cov 0.9.1";
const INSTRUMENTATION: &str = "cargo-test-no-run;workspace;all-targets;all-features;locked;instrument-coverage;exact-libtest-executables;partitioned-exact-tests";
const UNINSTRUMENTED: &str = "cargo-test-no-run;packages;all-targets;all-features;locked;uninstrumented;exact-libtest-executables;partitioned-exact-tests";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const COMMAND_OUTPUT_LIMIT: usize = 64 * 1024;
const JSON_LIMIT: u64 = 16 * 1024 * 1024;
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const PROFILE_LIMIT: u64 = 512 * 1024 * 1024;
const PROFILE_TOTAL_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
const PROFILE_COUNT_LIMIT: usize = 4096;
const RUNNER_LEDGER_LIMIT: u64 = 16 * 1024 * 1024;
const RUNNER_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound for one executable's `--list` run and its captured output.
const LIST_TIMEOUT: Duration = Duration::from_secs(120);
const LIST_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
/// Time kept between the partition's test deadline and its hosted job limit. It
/// covers tree termination and cleanup, fast refusal of the remaining Cargo
/// test executables, LCOV export, evidence writing and the diagnostics upload.
const EVIDENCE_RESERVE: Duration = Duration::from_secs(10 * 60);
const TEST_LOG_LIMIT: u64 = 32 * 1024 * 1024;
const RECENT_RESULT_LIMIT: usize = 20;
const PENDING_LINE_LIMIT: usize = 64 * 1024;

/// Every workspace package. The orchestrator requires `cargo metadata` to name
/// exactly this set, so the table cannot drift from the workspace.
pub const WORKSPACE_PACKAGES: [&str; 8] = [
    "kuru",
    "kuru-archive",
    "kuru-connectors",
    "kuru-core",
    "kuru-delivery",
    "kuru-memory",
    "kuru-platform",
    "kuru-runtime",
];

/// Whether a partition set runs under coverage instrumentation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Instrumented,
    Uninstrumented,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Instrumented => "instrumented",
            Self::Uninstrumented => "uninstrumented",
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "instrumented" => Ok(Self::Instrumented),
            "uninstrumented" => Ok(Self::Uninstrumented),
            _ => bail!("coverage mode {text:?} must be instrumented or uninstrumented"),
        }
    }

    fn contract(self) -> &'static str {
        match self {
            Self::Instrumented => INSTRUMENTATION,
            Self::Uninstrumented => UNINSTRUMENTED,
        }
    }
}

/// Hosted partition sets: OS label, mode and partition count. Workflow
/// matrices are pinned against this table, and it is the fail-closed
/// allowlist of hosted labels: a label or mode it does not name fails every
/// partition and merge. Windows on Arm runs the whole workspace suite
/// uninstrumented, as behavioral evidence that never enters a coverage gate,
/// until a pinned toolchain carries the fix for rust-lang/rust#150123.
pub const PARTITIONS: [(&str, Mode, u32); 5] = [
    ("ubuntu-latest", Mode::Instrumented, 8),
    ("macos-latest", Mode::Instrumented, 4),
    ("windows-latest", Mode::Instrumented, 8),
    ("ubuntu-24.04-arm", Mode::Uninstrumented, 3),
    ("windows-11-arm", Mode::Uninstrumented, 8),
];

/// The Rust host target every hosted OS label's partitions must report.
pub const OS_TARGETS: [(&str, &str); 5] = [
    ("ubuntu-latest", "x86_64-unknown-linux-gnu"),
    ("macos-latest", "aarch64-apple-darwin"),
    ("windows-latest", "x86_64-pc-windows-msvc"),
    ("ubuntu-24.04-arm", "aarch64-unknown-linux-gnu"),
    ("windows-11-arm", "aarch64-pc-windows-msvc"),
];

/// The OS label that local, by-hand partition runs use. It accepts any
/// partition count and host target.
pub const LOCAL_OS: &str = "local";

/// Test executables whose tests are listed but not run on one host target:
/// `(host target, artifact key, reason)`. An entry must name an executable of
/// the inventory; every partition records the reason, and the merge requires
/// the same reason everywhere.
pub const EXCLUDED_ARTIFACTS: [(&str, &str, &str); 0] = [];

/// Every workspace package, from [`WORKSPACE_PACKAGES`].
pub fn workspace_packages() -> BTreeSet<&'static str> {
    WORKSPACE_PACKAGES.into_iter().collect()
}

/// The partition count of a hosted OS label and mode, if the table has one.
pub fn partition_count(os: &str, mode: Mode) -> Option<u32> {
    PARTITIONS
        .iter()
        .find(|(label, table_mode, _)| *label == os && *table_mode == mode)
        .map(|(_, _, count)| *count)
}

/// Require a partition set to match the table, or to be a local run.
pub fn check_partitioning(os: &str, mode: Mode, count: u32) -> Result<()> {
    artifact_os_label(os)?;
    if os == LOCAL_OS {
        return Ok(());
    }
    let expected = partition_count(os, mode)
        .with_context(|| format!("no {} partition set is declared for {os}", mode.name()))?;
    ensure!(
        count == expected,
        "{os} {} partitions number {expected}, not {count}",
        mode.name()
    );
    Ok(())
}

/// The host target a hosted OS label must report, if it is not local.
pub fn os_target(os: &str) -> Option<&'static str> {
    OS_TARGETS
        .iter()
        .find(|(label, _)| *label == os)
        .map(|(_, target)| *target)
}

/// Validate the hosted OS label that names this OS's coverage artifacts.
pub fn artifact_os_label(label: &str) -> Result<&str> {
    ensure!(
        !label.is_empty()
            && !label.starts_with(['-', '.'])
            && !label.ends_with(['-', '.'])
            && label.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'.')),
        "coverage artifact OS label {label:?} must match [a-z0-9][a-z0-9.-]*[a-z0-9]"
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
#[serde(deny_unknown_fields)]
pub struct Inventory {
    schema: u32,
    workspace_packages: Vec<String>,
    /// Packages whose test targets were built: every workspace package when
    /// instrumented, the named packages when uninstrumented.
    scope: Vec<String>,
    artifacts: Vec<Artifact>,
}

/// Size and digest of the raw profiles a partition exported from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSummary {
    pub count: usize,
    pub bytes: u64,
    /// Digest of the sorted `name bytes sha256` lines of every profile.
    pub manifest_sha256: String,
}

/// The normalized LCOV a partition uploaded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LcovReceipt {
    pub bytes: u64,
    pub sha256: String,
    pub files: usize,
    pub lines_found: u64,
    pub lines_hit: u64,
}

/// The line export a partition uploaded: every instantiation's mapped and
/// covered lines, self-checked against that partition's own cargo-llvm-cov
/// `--summary-only` figures before the receipt is written.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LinesReceipt {
    pub bytes: u64,
    pub sha256: String,
    pub files: usize,
    pub instantiations: usize,
    /// cargo-llvm-cov's line total for this partition alone.
    pub count: u64,
    pub covered: u64,
}

/// One partition's evidence. Every field but the partition index, attempt,
/// plan, ledgers, profiles, LCOV and line export must agree across an OS's
/// partitions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: u32,
    pub partition: PartitionScheme,
    pub run_attempt: String,
    pub mode: Mode,
    pub scope: Vec<String>,
    pub source: String,
    pub tree: String,
    pub cargo_lock_sha256: String,
    pub rustc: String,
    pub cargo: String,
    pub target: String,
    pub cargo_llvm_cov: Option<String>,
    pub instrumentation: String,
    pub target_os: String,
    pub profile_env_sha256: String,
    pub inventory_sha256: String,
    pub plan_sha256: String,
    pub tests_sha256: String,
    pub runner_ledger_sha256: String,
    pub job_ledger_sha256: String,
    pub profiles: ProfileSummary,
    pub lcov: Option<LcovReceipt>,
    /// Required of an instrumented partition; the merge refuses a receipt
    /// without it.
    pub lines: Option<LinesReceipt>,
}

/// Evidence file names inside one uploaded `attempt-<n>` directory.
const RECEIPT_FILE: &str = "receipt.json";
const INVENTORY_FILE: &str = "inventory.json";
const PLAN_FILE: &str = "partition-plan.json";
const RUNNER_LEDGER_FILE: &str = "runner-ledger.jsonl";
const JOB_LEDGER_FILE: &str = "job-ledger.json";
const LCOV_FILE: &str = "coverage.lcov";
const LINES_FILE: &str = "coverage-lines.json";
/// The digested profile environment, whose digest is the receipt's
/// `profile_env_sha256`.
const PROFILE_ENV_FILE: &str = "profile-env.json";

/// The build and test environment a partition's receipt digests, by
/// `env:<name>` or `show-env:<name>`, with host-specific tokens neutralised.
/// Its values are Cargo and test settings and cargo-llvm-cov's coverage
/// environment (paths, flags and crate names), never credentials.
pub type ProfileEnv = BTreeMap<String, String>;

/// Parse a schema-bearing JSON document, naming a schema-1 file explicitly.
fn read_versioned<T: for<'de> Deserialize<'de>>(path: &Path, what: &str) -> Result<T> {
    let bytes = read_bounded(path, JSON_LIMIT)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    match value.get("schema").and_then(serde_json::Value::as_u64) {
        Some(1) => bail!(
            "{what} {} uses coverage schema 1 (package shards); schema {SCHEMA} partitions are required",
            path.display()
        ),
        Some(schema) if schema == u64::from(SCHEMA) => {}
        other => bail!(
            "{what} {} has unsupported coverage schema {other:?}",
            path.display()
        ),
    }
    serde_json::from_value(value).with_context(|| format!("parse {what} {}", path.display()))
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

/// Workspace member identity from `cargo metadata --no-deps`.
pub(crate) struct WorkspaceIdentity {
    /// Sorted member package names.
    pub packages: Vec<String>,
    /// Every member package and target name, which workspace outputs carry.
    pub names: BTreeSet<String>,
    /// Member package ids, as Cargo messages name them.
    pub ids: BTreeSet<String>,
}

pub(crate) fn workspace_identity(metadata_path: &Path) -> Result<WorkspaceIdentity> {
    let metadata = metadata(metadata_path)?;
    let members: BTreeSet<_> = metadata.workspace_members.iter().cloned().collect();
    let mut packages = Vec::new();
    let mut names = BTreeSet::new();
    for package in metadata
        .packages
        .iter()
        .filter(|package| members.contains(&package.id))
    {
        packages.push(package.name.clone());
        names.insert(package.name.clone());
        names.extend(package.targets.iter().map(|target| target.name.clone()));
    }
    packages.sort();
    Ok(WorkspaceIdentity {
        packages,
        names,
        ids: members,
    })
}

fn artifact_inventory(
    metadata: &Metadata,
    messages: &Path,
    target_dir: &Path,
    scope: &[String],
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
    let mut scope = scope.to_vec();
    scope.sort();
    scope.dedup();
    ensure!(!scope.is_empty(), "coverage inventory scope is empty");
    for package in &scope {
        ensure!(
            workspace_packages.binary_search(package).is_ok(),
            "coverage inventory scope names unknown package {package}"
        );
    }
    let inventory = Inventory {
        schema: SCHEMA,
        workspace_packages,
        scope,
        artifacts: artifacts.into_iter().collect(),
    };
    for artifact in inventory
        .artifacts
        .iter()
        .filter(|artifact| is_runnable(artifact))
    {
        ensure!(
            inventory.scope.binary_search(&artifact.package).is_ok(),
            "Cargo built a test executable outside the scope: {} {}",
            artifact.package,
            artifact.target_name
        );
    }
    for package in workspace
        .values()
        .filter(|package| inventory.scope.binary_search(&package.name).is_ok())
    {
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

/// Canonicalize the Cargo artifact inventory of `scope` into `output`.
pub fn write_inventory(
    metadata_path: &Path,
    messages: &Path,
    target_dir: &Path,
    scope: &[String],
    output: &Path,
) -> Result<()> {
    let inventory = artifact_inventory(&metadata(metadata_path)?, messages, target_dir, scope)?;
    write_json(output, &inventory)
}

fn is_runnable(artifact: &Artifact) -> bool {
    artifact.profile_test && artifact.executable.is_some()
}

fn read_inventory(path: &Path) -> Result<Inventory> {
    read_versioned(path, "coverage inventory")
}

/// The inventory's test executables, each with supported libtest semantics.
fn runnable_artifacts(inventory: &Inventory) -> Result<Vec<Artifact>> {
    let runnable: Vec<_> = inventory
        .artifacts
        .iter()
        .filter(|artifact| is_runnable(artifact))
        .cloned()
        .collect();
    ensure!(
        !runnable.is_empty(),
        "coverage inventory has no test executables"
    );
    let mut keys = BTreeSet::new();
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
        ensure!(
            keys.insert(partition::artifact_key(artifact)),
            "test executable identity {} is ambiguous",
            partition::artifact_key(artifact)
        );
    }
    Ok(runnable)
}

pub struct RunnerConfigOptions<'a> {
    pub root: &'a Path,
    pub host: &'a str,
    pub helper: &'a Path,
    pub inventory: &'a Path,
    pub partition: &'a PartitionScheme,
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
        partition,
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
        inventory_path.is_absolute(),
        "coverage manifests must be absolute"
    );
    ensure!(target_dir.is_absolute(), "coverage target must be absolute");
    ensure!(ledger.is_absolute(), "coverage ledger must be absolute");
    ensure!(
        diagnostics.is_absolute() && fs::symlink_metadata(diagnostics)?.file_type().is_dir(),
        "coverage diagnostics must be an absolute directory"
    );
    partition.validate()?;
    let now = unix_now()?;
    ensure!(
        *job_started <= now,
        "coverage job start {job_started} is in the future"
    );
    let deadline = shard_deadline(*job_started, *job_minutes)?;
    ensure!(
        deadline > now,
        "coverage partition deadline {deadline} passed before its tests started"
    );
    ensure!(
        !host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid Cargo host target"
    );
    let inventory = read_inventory(inventory_path)?;
    runnable_artifacts(&inventory)?;
    plan::check_exclusions(&EXCLUDED_ARTIFACTS, host, &inventory)?;
    let strings = [
        helper,
        root,
        target_dir,
        inventory_path,
        ledger,
        diagnostics,
    ]
    .map(|path| {
        path.to_str()
            .context("Cargo runner configuration path is not UTF-8")
    });
    let [helper, root, target_dir, inventory, ledger, diagnostics] = strings;
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
        "--host".to_owned(),
        (*host).to_owned(),
        "--partition".to_owned(),
        partition.index.to_string(),
        "--partitions".to_owned(),
        partition.count.to_string(),
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
    append_ledger_line(ledger, serde_json::to_vec(record)?)
}

/// Name the test executable behind the profiles its listing process wrote:
/// one spawn row per new `%p-%m` profile, so a later profile with the same
/// module signature names this executable.
fn record_listing_profiles(
    ledger: &Path,
    target_dir: &Path,
    executable: &Path,
    key: &str,
    before: &[String],
) -> Result<()> {
    let at = unix_now()?;
    for name in spawns::profile_names(target_dir)? {
        if before.binary_search(&name).is_ok() {
            continue;
        }
        if let Some(profile) = spawns::ProfileName::parse(&name) {
            let row = spawns::SpawnRecord::new(
                profile.pid,
                spawns::TEST_EXECUTABLE,
                &executable.to_string_lossy(),
                key,
                at,
            );
            append_ledger_line(ledger, serde_json::to_vec(&row)?)?;
        }
    }
    Ok(())
}

/// Append one line to the runner ledger within its size limit.
fn append_ledger_line(ledger: &Path, mut bytes: Vec<u8>) -> Result<()> {
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
    /// The Rust host target, which keys [`EXCLUDED_ARTIFACTS`].
    pub host: &'a str,
    pub partition: &'a PartitionScheme,
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
    /// The first `running N tests` announcement: the selection libtest ran.
    announced: Option<usize>,
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
        if self.announced.is_none()
            && let Some(count) = line
                .strip_prefix("running ")
                .and_then(|rest| {
                    rest.strip_suffix(" tests")
                        .or_else(|| rest.strip_suffix(" test"))
                })
                .filter(|count| {
                    !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit())
                })
                .and_then(|count| count.parse().ok())
        {
            self.announced = Some(count);
            return;
        }
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
    async fn terminate(&mut self) -> std::io::Result<()>;
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
    /// The tree exited; carries libtest's announced selection size.
    Exited(ExitStatus, Option<usize>),
    Stalled(Box<StallEvidence>),
    /// The Unix runner itself received a termination request.
    #[cfg(unix)]
    Interrupted(Box<InterruptEvidence>),
}

/// A termination request delivered to the Unix coverage runner.
///
/// The test executable runs in its own process group, so a terminal interrupt
/// or hangup reaches Cargo and this runner but not the test tree. The runner
/// therefore handles these requests itself and terminates the tree through its
/// retained owner before exiting.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerSignal {
    Interrupt,
    Terminate,
    Hangup,
}

#[cfg(unix)]
impl RunnerSignal {
    fn kind(self) -> tokio::signal::unix::SignalKind {
        use tokio::signal::unix::SignalKind;

        match self {
            Self::Interrupt => SignalKind::interrupt(),
            Self::Terminate => SignalKind::terminate(),
            Self::Hangup => SignalKind::hangup(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
            Self::Hangup => "SIGHUP",
        }
    }

    /// The conventional shell status of a process ended by this signal.
    pub fn exit_code(self) -> i32 {
        128 + self.kind().as_raw_value()
    }
}

/// Install the runner's handlers and return the first request they observe.
///
/// Call this before the test group exists: once installed, the handlers
/// replace the default disposition, so the runner can no longer die without
/// cleaning up the group.
#[cfg(unix)]
fn runner_signals() -> std::io::Result<impl std::future::Future<Output = RunnerSignal>> {
    use tokio::signal::unix::signal;

    let mut interrupt = signal(RunnerSignal::Interrupt.kind())?;
    let mut terminate = signal(RunnerSignal::Terminate.kind())?;
    let mut hangup = signal(RunnerSignal::Hangup.kind())?;
    Ok(async move {
        // `None` means the signal driver shut down, not that a signal arrived.
        tokio::select! {
            Some(()) = interrupt.recv() => RunnerSignal::Interrupt,
            Some(()) = terminate.recv() => RunnerSignal::Terminate,
            Some(()) = hangup.recv() => RunnerSignal::Hangup,
            else => std::future::pending().await,
        }
    })
}

#[cfg(unix)]
struct InterruptEvidence {
    signal: RunnerSignal,
    termination: String,
    cleanup: String,
    presence_after_reap: Option<String>,
}

/// The Unix runner stopped its test group after a termination request.
/// The caller exits with [`RunnerInterrupted::exit_code`].
#[cfg(unix)]
#[derive(Debug)]
pub struct RunnerInterrupted {
    signal: RunnerSignal,
    detail: String,
}

#[cfg(unix)]
impl RunnerInterrupted {
    pub fn exit_code(&self) -> i32 {
        self.signal.exit_code()
    }
}

#[cfg(unix)]
impl std::fmt::Display for RunnerInterrupted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

#[cfg(unix)]
impl std::error::Error for RunnerInterrupted {}

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
            let termination = format!("{:?}", process.terminate().await);
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
            Ok(Supervision::Exited(status, progress.announced))
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

/// One exact selection the runner starts for an executable.
struct Launch<'a> {
    executable: &'a Path,
    artifact: &'a Artifact,
    args: &'a [String],
    log: PathBuf,
    remaining: Duration,
    /// The runner ledger, named to test support through
    /// [`spawns::SPAWN_LEDGER_ENV`] so its detached children leave spawn rows.
    ledger: &'a Path,
}

/// The dispatcher's process boundary: listing an executable's tests and
/// running one supervised selection. Unit tests drive the dispatcher through a
/// fake; [`SystemLauncher`] starts real processes.
trait Launcher {
    /// Run `<executable> --list --format terse` from the current directory with
    /// the runner's environment and return its standard output.
    async fn list(&mut self, executable: &Path, timeout: Duration) -> Result<String>;
    /// Start and supervise one selection until it exits or the deadline.
    async fn run(&mut self, launch: &Launch<'_>) -> Result<Supervision>;
}

/// Real test processes: an owned Unix process group or a Windows native Job.
struct SystemLauncher;

impl Launcher for SystemLauncher {
    async fn list(&mut self, executable: &Path, timeout: Duration) -> Result<String> {
        let mut child = command::rooted(&std::env::current_dir()?, executable);
        child.args(["--list", "--format", "terse"]);
        let output = command::bounded_output(&mut child, timeout, LIST_OUTPUT_LIMIT)
            .await
            .with_context(|| format!("list the tests of {}", executable.display()))?;
        ensure!(
            output.status.success(),
            "{} --list failed with {}: {}",
            executable.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        String::from_utf8(output.stdout).context("libtest printed a non-UTF-8 test list")
    }

    async fn run(&mut self, launch: &Launch<'_>) -> Result<Supervision> {
        let Launch {
            executable,
            artifact,
            args,
            log,
            remaining,
            ledger,
        } = launch;
        #[cfg(not(windows))]
        let _ = artifact;
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
                async fn terminate(&mut self) -> std::io::Result<()> {
                    self.0.terminate()
                }
            }

            let mut spec = NativeSpawnSpec::new(executable.to_path_buf(), std::env::current_dir()?);
            // Only the verified memory test artifact exercises an owner that must
            // outlive its starter. Its test process remains in a kill-on-close Job,
            // but that Job permits the service's explicit native breakaway. All
            // other test artifacts retain the ordinary owned Job policy.
            if needs_independent_service_lifetime(artifact) {
                spec.lifetime = Lifetime::FixtureBreakawayJob;
            }
            spec.args = args.iter().map(OsString::from).collect();
            spec.environment = std::env::vars_os()
                .filter(|(name, _)| name != spawns::SPAWN_LEDGER_ENV)
                .collect();
            spec.environment
                .push((spawns::SPAWN_LEDGER_ENV.into(), ledger.as_os_str().into()));
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
                *remaining,
                RUNNER_CLEANUP_TIMEOUT,
            )
            .await?
        };
        #[cfg(unix)]
        let supervision = {
            // Handlers are installed before the test group exists.
            let interrupt = runner_signals()?;
            // The test inherits this runner's complete environment, including the
            // LLVM_PROFILE_FILE destination, working directory, stdin and stderr.
            let mut command = std::process::Command::new(executable);
            command
                .args(args.iter())
                .env(spawns::SPAWN_LEDGER_ENV, ledger)
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit());
            supervise_group(
                command,
                tokio::io::stdout(),
                log.clone(),
                *remaining,
                RUNNER_CLEANUP_TIMEOUT,
                interrupt,
            )
            .await?
        };
        Ok(supervision)
    }
}

/// Raw profiles directly in the target root.
fn count_profiles(target: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(target)? {
        if entry?.path().extension() == Some(OsStr::new("profraw")) {
            count += 1;
        }
    }
    Ok(count)
}

/// The time left before the deadline, or `None` once it has passed.
fn remaining_until(deadline: u64) -> Result<Option<Duration>> {
    let now = unix_now()?;
    Ok((now < deadline).then(|| Duration::from_secs(deadline - now)))
}

/// Close a record with its end time and profile count and append it.
fn finish_record(record: &mut RunnerRecord, ledger: &Path, target_dir: &Path) -> Result<()> {
    record.finished = unix_now()?.max(record.started);
    record.profraw_after = count_profiles(target_dir)?;
    append_runner_record(ledger, record)
}

/// Dispatch one Cargo-selected test executable through the real processes.
pub async fn dispatch_test(options: &DispatchOptions<'_>) -> Result<Option<ExitStatus>> {
    let cwd = std::env::current_dir()?;
    dispatch_with(&mut SystemLauncher, options, &cwd, &EXCLUDED_ARTIFACTS).await
}

/// Dispatch from Cargo's working directory `current` through `launcher`.
async fn dispatch_with<L: Launcher>(
    launcher: &mut L,
    options: &DispatchOptions<'_>,
    current: &Path,
    exclusions: &[(&str, &str, &str)],
) -> Result<Option<ExitStatus>> {
    let DispatchOptions {
        root,
        inventory: inventory_path,
        host,
        partition,
        target_dir,
        ledger,
        diagnostics,
        deadline,
        executable,
        args,
    } = options;
    ensure!(root.is_absolute(), "coverage root must be absolute");
    ensure!(
        inventory_path.is_absolute(),
        "coverage manifests must be absolute"
    );
    ensure!(target_dir.is_absolute(), "coverage target must be absolute");
    ensure!(ledger.is_absolute(), "coverage ledger must be absolute");
    ensure!(
        diagnostics.is_absolute() && fs::symlink_metadata(diagnostics)?.file_type().is_dir(),
        "coverage diagnostics must be an absolute directory"
    );
    partition.validate()?;
    let executable_path = if executable.is_absolute() {
        executable.to_path_buf()
    } else {
        current.join(executable)
    };
    ensure!(
        fs::symlink_metadata(&executable_path)?
            .file_type()
            .is_file(),
        "Cargo runner executable is not a regular file"
    );
    let executable = normalized_relative(target_dir, &executable_path, "runner executable")?;
    let inventory = read_inventory(inventory_path)?;
    let artifacts = runnable_artifacts(&inventory)?;
    let matching: Vec<_> = artifacts
        .iter()
        .filter(|artifact| artifact.executable.as_deref() == Some(executable.as_str()))
        .collect();
    let [artifact] = matching[..] else {
        ensure!(
            matching.is_empty(),
            "Cargo test executable identity is ambiguous"
        );
        bail!("Cargo invoked unknown test executable {executable}");
    };
    let cwd = normalized_relative(root, current, "runner directory")?;
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
    let key = partition::artifact_key(artifact);
    let started = unix_now()?;
    let profraw_before = count_profiles(target_dir)?;
    let empty = partition::list_sha256(&[]);
    let mut record = RunnerRecord {
        schema: SCHEMA,
        executable: executable.clone(),
        artifact: key.clone(),
        action: plan::DEADLINE.to_owned(),
        cwd,
        args,
        listed: Vec::new(),
        list_sha256: empty.clone(),
        assigned: 0,
        assigned_sha256: empty,
        reason: None,
        invocations: Vec::new(),
        started,
        finished: started,
        profraw_before,
        profraw_after: profraw_before,
        success: false,
        status_code: None,
    };
    let Some(remaining) = remaining_until(*deadline)? else {
        finish_record(&mut record, ledger, target_dir)?;
        bail!("coverage partition deadline {deadline} passed before starting {executable}");
    };

    // Every partition lists every executable: the lists prove completeness,
    // and in an instrumented partition the list run leaves each object a profile.
    let list_started = unix_now()?;
    let before_list = spawns::profile_names(target_dir)?;
    let listed = match launcher
        .list(&executable_path, remaining.min(LIST_TIMEOUT))
        .await
        .and_then(|text| partition::parse_libtest_list(&text))
    {
        Ok(listed) => listed,
        Err(error) => {
            record.action = plan::LIST.to_owned();
            finish_record(&mut record, ledger, target_dir)?;
            return Err(error.context(format!("list the tests of {executable}")));
        }
    };
    record_listing_profiles(ledger, target_dir, &executable_path, &key, &before_list)?;
    record.list_sha256 = partition::list_sha256(&listed);
    record.invocations.push(InvocationRecord {
        kind: plan::LIST.to_owned(),
        names: listed.len(),
        names_sha256: record.list_sha256.clone(),
        announced: None,
        started: list_started,
        finished: unix_now()?,
        status_code: Some(0),
    });
    let excluded = plan::excluded_reason(exclusions, host, &key);
    let assigned = if excluded.is_some() {
        Vec::new()
    } else {
        partition.assigned(&key, &listed)
    };
    record.listed = listed;
    record.assigned = assigned.len();
    record.assigned_sha256 = partition::list_sha256(&assigned);
    if let Some(reason) = excluded {
        eprintln!("coverage runner: {executable} is excluded on {host}: {reason}");
        record.reason = Some(reason);
        record.action = plan::EXCLUDE.to_owned();
    } else if assigned.is_empty() {
        record.action = plan::OMIT.to_owned();
    }
    if assigned.is_empty() {
        record.success = true;
        record.status_code = Some(0);
        finish_record(&mut record, ledger, target_dir)?;
        return Ok(None);
    }

    record.action = plan::RUN.to_owned();
    let program = executable_path
        .to_str()
        .context("Cargo runner executable path is not UTF-8")?;
    let chunks = match partition::chunk(
        program,
        &["--exact"],
        &assigned,
        partition::COMMAND_LINE_BUDGET,
    ) {
        Ok(chunks) => chunks,
        Err(error) => {
            finish_record(&mut record, ledger, target_dir)?;
            return Err(error);
        }
    };
    let chunk_count = chunks.len();
    let mut last = None;
    // A failing chunk does not stop the executable's remaining chunks, so the
    // assignment keeps the diagnostics a single `--no-fail-fast` run gave.
    let mut failed: Option<ExitStatus> = None;
    for (index, names) in chunks.into_iter().enumerate() {
        let Some(remaining) = remaining_until(*deadline)? else {
            if let Some(status) = failed {
                eprintln!(
                    "coverage runner: deadline {deadline} passed before chunk {} of {chunk_count} of {executable} after a failed chunk",
                    index + 1
                );
                record.status_code = status.code();
                finish_record(&mut record, ledger, target_dir)?;
                return Ok(Some(status));
            }
            record.action = plan::DEADLINE.to_owned();
            finish_record(&mut record, ledger, target_dir)?;
            bail!(
                "coverage partition deadline {deadline} passed before chunk {} of {chunk_count} of {executable}",
                index + 1
            );
        };
        let label = format!("{}.exact-{}", diagnostic_name(&executable), index + 1);
        let log = diagnostics.join(format!("{label}.stdout.log"));
        let mut selection = Vec::with_capacity(names.len() + 1);
        selection.push("--exact".to_owned());
        selection.extend(names.iter().cloned());
        let chunk_started = unix_now()?;
        let supervision = launcher
            .run(&Launch {
                executable: &executable_path,
                artifact,
                args: &selection,
                log: log.clone(),
                remaining,
                ledger,
            })
            .await;
        let supervision = match supervision {
            Ok(supervision) => supervision,
            Err(error) => {
                finish_record(&mut record, ledger, target_dir)?;
                return Err(error);
            }
        };
        let mut invocation = InvocationRecord {
            kind: plan::EXACT.to_owned(),
            names: names.len(),
            names_sha256: partition::list_sha256(&names),
            announced: None,
            started: chunk_started,
            finished: unix_now()?,
            status_code: None,
        };
        let (status, announced) = match supervision {
            Supervision::Exited(status, announced) => (status, announced),
            #[cfg(unix)]
            Supervision::Interrupted(evidence) => {
                record.invocations.push(invocation);
                finish_record(&mut record, ledger, target_dir)?;
                let InterruptEvidence {
                    signal,
                    termination,
                    cleanup,
                    presence_after_reap,
                } = *evidence;
                return Err(RunnerInterrupted {
                    signal,
                    detail: format!(
                        "coverage runner received {} while {executable} was running; termination={termination}; cleanup={cleanup}; presence_after_reap={presence_after_reap:?}",
                        signal.name()
                    ),
                }
                .into());
            }
            Supervision::Stalled(evidence) => {
                record.invocations.push(invocation);
                finish_record(&mut record, ledger, target_dir)?;
                let report = stall_report(artifact, &executable, *deadline, evidence, &log)?;
                let path = diagnostics.join(format!("{label}.stall.json"));
                write_json(&path, &report)?;
                bail!(
                    "coverage partition deadline reached while {executable} was running; unfinished tests: {:?}; latest results: {:?}; evidence: {}",
                    report.unfinished_tests,
                    report.recent_results.last(),
                    path.display()
                );
            }
        };
        invocation.announced = announced;
        invocation.status_code = status.code();
        record.invocations.push(invocation);
        if !status.success() {
            failed.get_or_insert(status);
            continue;
        }
        if announced != Some(names.len()) {
            finish_record(&mut record, ledger, target_dir)?;
            bail!(
                "libtest announced {announced:?} tests for a {}-name selection of {executable}",
                names.len()
            );
        }
        last = Some(status);
    }
    if let Some(status) = failed {
        record.status_code = status.code();
        finish_record(&mut record, ledger, target_dir)?;
        return Ok(Some(status));
    }
    record.success = true;
    record.status_code = Some(0);
    finish_record(&mut record, ledger, target_dir)?;
    Ok(last)
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
    ///
    /// Settling is bounded by `settle_bound` and never outlives the caller's
    /// `deadline`: a settle cut short by that deadline reports `TimedOut`, as
    /// the wait it belongs to would, so the caller's cleanup still applies.
    async fn settle(&mut self, deadline: tokio::time::Instant) -> std::io::Result<ExitStatus> {
        use kuru_platform::unix::{GroupPresence, Reap, Termination};

        let bound = self.settle_bound;
        let own = tokio::time::Instant::now() + bound;
        let (limit, kind, within) = if deadline < own {
            (
                deadline,
                std::io::ErrorKind::TimedOut,
                "before the wait deadline",
            )
        } else {
            (own, std::io::ErrorKind::Other, "within the settle bound")
        };
        let expired = |what: &str| {
            std::io::Error::new(
                kind,
                format!("owned test process group {what} {within} ({bound:?})"),
            )
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
        let mut listing = self.owner.permission_listing(limit.into_std());
        loop {
            let presence = listing.resolve(self.owner.presence_after_reap()).await;
            self.presence = Some(format!("{presence:?}"));
            match presence {
                GroupPresence::Absent | GroupPresence::Recycled => return Ok(status),
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
///
/// `interrupt` resolves when the runner itself is asked to stop. Supervision is
/// then abandoned (its wait is cancellation-safe) and the group is terminated,
/// reaped and confirmed absent through the same owner as a stall.
#[cfg(unix)]
async fn supervise_group<W, S>(
    command: std::process::Command,
    relay: W,
    log: PathBuf,
    remaining: Duration,
    cleanup_bound: Duration,
    interrupt: S,
) -> Result<Supervision>
where
    W: tokio::io::AsyncWrite + Unpin,
    S: std::future::Future<Output = RunnerSignal>,
{
    let mut child = GroupProcess::spawn(command, cleanup_bound)?;
    let output = match child.take_stdout() {
        Ok(output) => output,
        Err(error) => {
            let termination = child.terminate().await;
            let cleanup = child.wait(cleanup_bound).await;
            bail!(
                "Cargo test process has no stdout pipe: {error}; termination={termination:?}; cleanup={cleanup:?}"
            );
        }
    };
    let signal = {
        let supervised = supervise(&mut child, output, relay, log, remaining, cleanup_bound);
        tokio::select! {
            biased;
            signal = interrupt => signal,
            supervision = supervised => return supervision,
        }
    };
    let termination = format!("{:?}", child.terminate().await);
    let cleanup = format!("{:?}", child.wait(cleanup_bound).await);
    Ok(Supervision::Interrupted(Box::new(InterruptEvidence {
        signal,
        termination,
        cleanup,
        // Recorded only after the cleanup wait has had its chance to reap.
        presence_after_reap: child.presence_after_reap(),
    })))
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
                // A reaped root still settles: a cancelled wait may have been
                // dropped between the reap and the absence confirmation.
                RootState::Exited | RootState::Reaped(_) => return self.settle(deadline).await,
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

    async fn terminate(&mut self) -> std::io::Result<()> {
        use kuru_platform::unix::Termination;

        let limit = tokio::time::Instant::now() + self.settle_bound;
        loop {
            match self.owner.terminate_before_reap() {
                // The transition may already have been consumed by a settling exit.
                Termination::Signalled(_) | Termination::InvalidPhase => return Ok(()),
                Termination::Interrupted => {}
                Termination::Disarmed(reason) => return Err(disarmed(reason)),
            }
            if tokio::time::Instant::now() >= limit {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "owned test process group observation was interrupted",
                ));
            }
            tokio::time::sleep(GROUP_POLL).await;
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

/// Validate a runner ledger file against its inventory and partition.
pub fn validate_run_ledger(
    inventory_path: &Path,
    partition: &PartitionScheme,
    host: &str,
    ledger: &Path,
) -> Result<plan::PartitionPlan> {
    let inventory = read_inventory(inventory_path)?;
    let records = plan::read_ledger(ledger)?;
    plan::validate_run_ledger(&inventory, partition, host, &EXCLUDED_ARTIFACTS, &records)
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

/// The exact source and toolchain identity of this runner. `llvm_cov` names
/// the pinned cargo-llvm-cov executable of an instrumented partition.
async fn identity(
    root: &Path,
    expected_source: &str,
    llvm_cov: Option<&Path>,
) -> Result<ReceiptIdentity> {
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
    let cargo_llvm_cov = match llvm_cov {
        Some(llvm_cov) => {
            let version =
                checked_output(root, llvm_cov.as_os_str(), &["llvm-cov", "--version"]).await?;
            ensure!(
                version == LLVM_COV_VERSION,
                "expected {LLVM_COV_VERSION}, got {version}"
            );
            Some(version)
        }
        None => None,
    };
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
    cargo_llvm_cov: Option<String>,
    target_os: String,
}

/// Reject a different or modified tracked source before compiling.
pub async fn verify_source(
    root: &Path,
    expected_source: &str,
    llvm_cov: Option<&Path>,
) -> Result<()> {
    identity(root, expected_source, llvm_cov).await.map(drop)
}

/// Hash one raw profile without following links, within the size bound.
fn hash_profile(path: &Path) -> Result<(u64, String)> {
    let metadata = fs::symlink_metadata(path)?;
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
    let mut input = File::open(path)?;
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
            path.display()
        );
        hasher.update(&buffer[..length]);
    }
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((bytes, sha256))
}

/// Bound and summarize the raw profiles in the target root. An instrumented
/// partition must have written some; an uninstrumented one must have none.
fn profile_summary(directory: &Path, mode: Mode) -> Result<ProfileSummary> {
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
        "coverage partition produced too many profiles"
    );
    let mut manifest = String::new();
    let mut total = 0_u64;
    let mut nonempty = 0;
    for path in &candidates {
        let (bytes, sha256) = hash_profile(path)?;
        total = total
            .checked_add(bytes)
            .context("profile total size overflow")?;
        if bytes > 0 {
            nonempty += 1;
        }
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .context("profile lacks a UTF-8 name")?;
        manifest.push_str(&format!("{name} {bytes} {sha256}\n"));
    }
    ensure!(
        total <= PROFILE_TOTAL_LIMIT,
        "coverage partition profiles exceed total size limit"
    );
    match mode {
        Mode::Instrumented => ensure!(
            nonempty > 0,
            "coverage partition produced no nonempty raw profiles"
        ),
        Mode::Uninstrumented => ensure!(
            candidates.is_empty(),
            "an uninstrumented partition produced raw profiles"
        ),
    }
    Ok(ProfileSummary {
        count: candidates.len(),
        bytes: total,
        manifest_sha256: archive::digest(manifest.as_bytes()),
    })
}

/// Remove compile-phase profiles before the partition's tests run.
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

pub struct ReceiptOptions<'a> {
    pub root: &'a Path,
    pub mode: Mode,
    pub partition: &'a PartitionScheme,
    pub inventory: &'a Path,
    pub ledger: &'a Path,
    pub job_ledger: &'a Path,
    /// The target root holding this partition's raw profiles.
    pub profiles: &'a Path,
    /// The normalized LCOV of an instrumented partition.
    pub lcov: Option<&'a Path>,
    /// The self-checked line export of an instrumented partition.
    pub lines: Option<&'a Path>,
    pub run_attempt: &'a str,
    pub expected_source: &'a str,
    pub llvm_cov: Option<&'a Path>,
    /// Uploaded beside the receipt, which carries its digest.
    pub profile_env: &'a ProfileEnv,
    pub output: &'a Path,
}

/// Bind the partition's plan, ledgers and coverage to its exact identity and
/// write the uploaded `attempt-<n>` evidence directory.
pub async fn write_receipt(options: &ReceiptOptions<'_>) -> Result<()> {
    let ReceiptOptions {
        root,
        run_attempt,
        expected_source,
        llvm_cov,
        ..
    } = options;
    canonical_attempt(run_attempt)
        .context("coverage run attempt must be a positive integer without leading zeros")?;
    let observed = identity(root, expected_source, *llvm_cov).await?;
    write_evidence(options, observed)
}

/// Write the evidence of [`write_receipt`] for an observed identity.
fn write_evidence(options: &ReceiptOptions<'_>, observed: ReceiptIdentity) -> Result<()> {
    let ReceiptOptions {
        mode,
        partition,
        inventory: inventory_path,
        ledger,
        job_ledger,
        profiles,
        lcov,
        lines: lines_path,
        run_attempt,
        profile_env,
        output,
        ..
    } = options;
    let attempt = canonical_attempt(run_attempt)
        .context("coverage run attempt must be a positive integer without leading zeros")?;
    partition.validate()?;
    let inventory = read_inventory(inventory_path)?;
    if *mode == Mode::Instrumented {
        ensure!(
            inventory.scope == inventory.workspace_packages,
            "an instrumented partition must build the whole workspace"
        );
    }
    ensure!(
        observed.cargo_llvm_cov.is_some() == (*mode == Mode::Instrumented),
        "coverage tool identity does not match the {} mode",
        mode.name()
    );
    let records = plan::read_ledger(ledger)?;
    let plan = plan::validate_run_ledger(
        &inventory,
        partition,
        &observed.target,
        &EXCLUDED_ARTIFACTS,
        &records,
    )?;
    let runner_ledger = read_bounded(ledger, RUNNER_LEDGER_LIMIT)?;
    let job_ledger_bytes = read_bounded(job_ledger, JSON_LIMIT)?;
    ledger::JobLedger::parse(&job_ledger_bytes).context("job ledger")?;
    let profile_summary = profile_summary(profiles, *mode)?;
    let lcov_bytes = match (mode, lcov) {
        (Mode::Instrumented, Some(path)) => Some(read_bounded(path, lcov::LCOV_LIMIT)?),
        (Mode::Uninstrumented, None) => None,
        _ => bail!("an LCOV report is required exactly for an instrumented partition"),
    };
    let lcov_receipt = match &lcov_bytes {
        Some(bytes) => {
            let parsed = lcov::Lcov::parse(
                std::str::from_utf8(bytes).context("partition LCOV is not UTF-8")?,
            )?;
            parsed.require_relative()?;
            let totals = parsed.totals();
            Some(LcovReceipt {
                bytes: bytes.len() as u64,
                sha256: archive::digest(bytes),
                files: totals.files,
                lines_found: totals.lines_found,
                lines_hit: totals.lines_hit,
            })
        }
        None => None,
    };
    let lines_bytes = match (mode, lines_path) {
        (Mode::Instrumented, Some(path)) => Some(read_bounded(path, lines::LINES_LIMIT)?),
        (Mode::Uninstrumented, None) => None,
        _ => bail!("a line export is required exactly for an instrumented partition"),
    };
    let lines_receipt = match &lines_bytes {
        Some(bytes) => {
            let export = lines::LineExport::parse(bytes).context("partition line export")?;
            let figures = export.figures()?;
            Some(LinesReceipt {
                bytes: bytes.len() as u64,
                sha256: archive::digest(bytes),
                files: export.files.len(),
                instantiations: export.instantiation_count(),
                count: figures.total.count,
                covered: figures.total.covered,
            })
        }
        None => None,
    };
    let receipt = Receipt {
        schema: SCHEMA,
        partition: (*partition).clone(),
        run_attempt: (*run_attempt).to_owned(),
        mode: *mode,
        scope: inventory.scope.clone(),
        source: observed.source,
        tree: observed.tree,
        cargo_lock_sha256: observed.cargo_lock_sha256,
        rustc: observed.rustc,
        cargo: observed.cargo,
        target: observed.target,
        cargo_llvm_cov: observed.cargo_llvm_cov,
        instrumentation: mode.contract().to_owned(),
        target_os: observed.target_os,
        profile_env_sha256: digest_json(profile_env)?,
        inventory_sha256: digest_json(&inventory)?,
        plan_sha256: digest_json(&plan)?,
        tests_sha256: plan::tests_sha256(&plan)?,
        runner_ledger_sha256: archive::digest(&runner_ledger),
        job_ledger_sha256: archive::digest(&job_ledger_bytes),
        profiles: profile_summary,
        lcov: lcov_receipt,
        lines: lines_receipt,
    };
    // The uploaded artifact root holds one `attempt-<n>` directory, so the
    // evidence names its attempt whether download-artifact extracts it into
    // the destination itself or into a directory named after the artifact.
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::create_dir(output).with_context(|| format!("create {}", output.display()))?;
    let output = &output.join(attempt_directory(attempt));
    fs::create_dir(output).with_context(|| format!("create {}", output.display()))?;
    write_json(&output.join(INVENTORY_FILE), &inventory)?;
    write_json(&output.join(PLAN_FILE), &plan)?;
    write_json(&output.join(PROFILE_ENV_FILE), profile_env)?;
    write_new(&output.join(RUNNER_LEDGER_FILE), &runner_ledger)?;
    write_new(&output.join(JOB_LEDGER_FILE), &job_ledger_bytes)?;
    if let Some(bytes) = &lcov_bytes {
        write_new(&output.join(LCOV_FILE), bytes)?;
    }
    if let Some(bytes) = &lines_bytes {
        write_new(&output.join(LINES_FILE), bytes)?;
    }
    write_json(&output.join(RECEIPT_FILE), &receipt)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
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

    #[test]
    fn tables_cover_the_workspace_and_hosted_labels() {
        let unique = workspace_packages();
        assert_eq!(unique.len(), WORKSPACE_PACKAGES.len(), "a package repeats");
        assert!(WORKSPACE_PACKAGES.is_sorted());
        // The root workspace's members are exactly the table's packages.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
        let mut members: Vec<String> = manifest["workspace"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| {
                let path = root.join(member.as_str().unwrap()).join("Cargo.toml");
                let package: toml::Value =
                    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
                package["package"]["name"].as_str().unwrap().to_owned()
            })
            .collect();
        members.sort();
        assert_eq!(members, WORKSPACE_PACKAGES);

        let labels: BTreeSet<_> = PARTITIONS.iter().map(|(label, _, _)| *label).collect();
        assert_eq!(
            labels.len(),
            PARTITIONS.len(),
            "one partition set per label"
        );
        for (label, mode, count) in PARTITIONS {
            assert_eq!(artifact_os_label(label).unwrap(), label);
            assert!(os_target(label).is_some(), "{label} has no host target");
            assert!(PartitionScheme::new(count, count).is_ok(), "{label}");
            assert_eq!(partition_count(label, mode), Some(count));
            check_partitioning(label, mode, count).unwrap();
            let error = check_partitioning(label, mode, count + 1)
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("number {count}")), "{error}");
        }
        assert_eq!(
            OS_TARGETS.map(|(label, _)| label).to_vec(),
            PARTITIONS.map(|(label, _, _)| label).to_vec()
        );
        // macOS fits the five-job cap beside its install job; Windows and
        // Ubuntu stay inside the account's concurrency.
        assert_eq!(partition_count("macos-latest", Mode::Instrumented), Some(4));
        assert_eq!(
            partition_count("windows-latest", Mode::Instrumented),
            Some(8)
        );
        assert!((8..=12).contains(&partition_count("ubuntu-latest", Mode::Instrumented).unwrap()));
        assert_eq!(
            partition_count("ubuntu-24.04-arm", Mode::Uninstrumented),
            Some(3)
        );
        assert_eq!(partition_count("ubuntu-latest", Mode::Uninstrumented), None);
        // Windows on Arm runs the Windows partition count uninstrumented; its
        // instrumented set stays held (rust-lang/rust#150123), so an
        // instrumented run there is refused, not quietly downgraded.
        assert_eq!(
            partition_count("windows-11-arm", Mode::Uninstrumented),
            partition_count("windows-latest", Mode::Instrumented)
        );
        assert_eq!(os_target("windows-11-arm"), Some("aarch64-pc-windows-msvc"));
        check_partitioning("windows-11-arm", Mode::Uninstrumented, 8).unwrap();
        let error = check_partitioning("windows-11-arm", Mode::Instrumented, 8)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no instrumented partition set is declared for windows-11-arm"),
            "{error}"
        );
        check_partitioning(LOCAL_OS, Mode::Uninstrumented, 5).unwrap();
        // Unknown hosted labels, including near-misses of the Windows ones,
        // are refused in either mode.
        for label in ["windows-2025", "windows-11-arm64", "windows-latest-arm"] {
            assert_eq!(os_target(label), None, "{label}");
            for mode in [Mode::Instrumented, Mode::Uninstrumented] {
                let error = check_partitioning(label, mode, 8).unwrap_err().to_string();
                assert!(
                    error.contains(&format!(
                        "no {} partition set is declared for {label}",
                        mode.name()
                    )),
                    "{error}"
                );
            }
        }
        assert_eq!(os_target(LOCAL_OS), None);
        assert!(EXCLUDED_ARTIFACTS.is_empty());
        for mode in [Mode::Instrumented, Mode::Uninstrumented] {
            assert_eq!(Mode::parse(mode.name()).unwrap(), mode);
        }
        assert!(Mode::parse("partial").is_err());
        assert_ne!(
            Mode::Instrumented.contract(),
            Mode::Uninstrumented.contract()
        );
    }

    #[test]
    fn artifact_os_labels_are_runner_names() {
        for label in [
            "ubuntu-latest",
            "macos-latest",
            "windows-2025",
            "windows-latest",
            "ubuntu-24.04-arm",
            "windows-11-arm",
            LOCAL_OS,
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
            ".hidden",
            "trailing.",
            "-flag",
            "dash-",
        ] {
            let error = artifact_os_label(label).unwrap_err().to_string();
            assert!(error.contains("[a-z0-9]"), "{label}: {error}");
        }
    }

    #[test]
    fn schema_one_files_are_refused_by_name() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("inventory.json");
        fs::write(
            &path,
            br#"{"schema":1,"workspace_packages":[],"artifacts":[]}"#,
        )
        .unwrap();
        let error = read_inventory(&path).unwrap_err().to_string();
        assert!(
            error.contains("coverage schema 1 (package shards)"),
            "{error}"
        );
        fs::write(&path, br#"{"schema":3}"#).unwrap();
        let error = read_inventory(&path).unwrap_err().to_string();
        assert!(
            error.contains("unsupported coverage schema Some(3)"),
            "{error}"
        );
        fs::write(&path, br#"{"schema":2,"unknown":true}"#).unwrap();
        assert!(read_inventory(&path).is_err());
        let ledger = temp.path().join("ledger.jsonl");
        fs::write(&ledger, b"{\"schema\":1}\n").unwrap();
        let error = plan::read_ledger(&ledger).unwrap_err().to_string();
        assert!(error.contains("schema 1 (package shards)"), "{error}");
        fs::write(&ledger, b"{}\n").unwrap();
        let error = plan::read_ledger(&ledger).unwrap_err().to_string();
        assert!(error.contains("unsupported schema None"), "{error}");
        fs::write(&ledger, b"nonsense\n").unwrap();
        assert!(plan::read_ledger(&ledger).is_err());
    }

    #[tokio::test]
    async fn receipts_require_a_canonical_run_attempt() {
        let temp = TempDir::new().unwrap();
        let path = temp.path();
        let partition = PartitionScheme::new(1, 1).unwrap();
        for attempt in ["", "0", "03", "x"] {
            let error = write_receipt(&ReceiptOptions {
                root: path,
                mode: Mode::Instrumented,
                partition: &partition,
                inventory: path,
                ledger: path,
                job_ledger: path,
                profiles: path,
                lcov: None,
                lines: None,
                run_attempt: attempt,
                expected_source: "HEAD",
                llvm_cov: None,
                profile_env: &ProfileEnv::new(),
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
        async fn terminate(&mut self) -> std::io::Result<()> {
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
        assert!(matches!(supervision, Supervision::Exited(status, _) if status.success()));
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
            std::future::pending(),
        )
        .await
        .unwrap();
        let Supervision::Exited(status, _) = supervision else {
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
        process.terminate().await.unwrap();
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
        process.terminate().await.unwrap();
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
            std::future::pending(),
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
            std::future::pending(),
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

    /// Re-executed by the signal test below; inert in an ordinary test run.
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_runner_signal_child() {
        let Some(directory) = std::env::var_os("KURU_COVERAGE_SIGNAL_CHILD") else {
            return;
        };
        // The production handlers, installed before the group exists. The
        // readiness line names the group only after both are in place.
        let interrupt = runner_signals().unwrap();
        let supervision = supervise_group(
            group_command("sleep 300 & printf 'ready %s\\n' $$; wait"),
            tokio::io::stdout(),
            PathBuf::from(directory).join("signal.stdout.log"),
            GROUP_BOUND * 3,
            GROUP_BOUND,
            interrupt,
        )
        .await
        .unwrap();
        let Supervision::Interrupted(evidence) = supervision else {
            panic!("an interrupted runner reported its group as finished");
        };
        println!(
            "interrupted {} termination={} presence_after_reap={:?}",
            evidence.signal.name(),
            evidence.termination,
            evidence.presence_after_reap
        );
        std::process::exit(evidence.signal.exit_code());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_runner_signal_terminates_its_group_and_exits_with_the_signal() {
        use kuru_platform::unix::observe_group_after_reap;
        use rustix::process::{Pid, Signal, kill_process};
        use std::io::BufRead as _;

        assert_eq!(RunnerSignal::Interrupt.exit_code(), 130);
        assert_eq!(RunnerSignal::Terminate.exit_code(), 143);
        assert_eq!(RunnerSignal::Hangup.exit_code(), 129);

        let temp = TempDir::new().unwrap();
        // The environment, including LLVM_PROFILE_FILE, is inherited unchanged.
        let mut runner = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "coverage::tests::unix_runner_signal_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("KURU_COVERAGE_SIGNAL_CHILD", temp.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        // Drain the runner's stdout on a thread so waits below stay bounded.
        let (lines, received) = std::sync::mpsc::channel();
        let stdout = runner.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        let mut seen = Vec::new();
        let group = loop {
            match received.recv_timeout(GROUP_BOUND) {
                Ok(line) => {
                    // libtest's unterminated test name may precede the marker.
                    if let Some((_, group)) = line.rsplit_once("ready ") {
                        break group.trim().parse::<i32>().unwrap();
                    }
                    seen.push(line);
                }
                Err(error) => {
                    let _ = runner.kill();
                    let _ = runner.wait();
                    panic!("runner never reported its group ({error}); output: {seen:?}");
                }
            }
        };
        // The runner is our unreaped child, so its identity is still ours.
        kill_process(Pid::from_child(&runner), Signal::TERM).unwrap();
        let limit = std::time::Instant::now() + GROUP_BOUND * 2;
        let status = loop {
            if let Some(status) = runner.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= limit {
                let _ = runner.kill();
                let _ = runner.wait();
                panic!("runner did not exit after SIGTERM; output: {seen:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        reader.join().unwrap();
        seen.extend(received.try_iter());
        assert_eq!(status.code(), Some(143), "{status:?}; output: {seen:?}");
        assert!(
            seen.iter().any(|line| line
                == "interrupted SIGTERM termination=Ok(()) presence_after_reap=Some(\"Absent\")"),
            "{seen:?}"
        );
        // Observation only: the runner reaped the root and saw the group absent.
        // Another user may have reused the number since; ours must not remain.
        let observed = observe_group_after_reap(group.unsigned_abs());
        assert!(observed.none_of_ours(), "{observed}");
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
    fn inventories_are_scoped_to_their_packages() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let target = temp.path().join("target");
        workspace(&root, &target);
        let metadata_path = temp.path().join("metadata.json");
        write(&metadata_path, &metadata_value(&root));
        let identity = workspace_identity(&metadata_path).unwrap();
        assert_eq!(identity.packages, ["alpha", "common"]);
        assert!(identity.names.contains("alpha") && identity.ids.contains("common-id"));
        let full = temp.path().join("full.json");
        cargo_messages(
            &full,
            &[
                artifact(&root, &target, "alpha", "alpha", "normal", false),
                artifact(&root, &target, "alpha", "alpha", "test", true),
                artifact(&root, &target, "common", "common", "normal", false),
                artifact(&root, &target, "common", "common", "test", true),
            ],
        );
        let scoped = temp.path().join("scoped.json");
        cargo_messages(
            &scoped,
            &[
                artifact(&root, &target, "alpha", "alpha", "test", true),
                // A workspace dependency's library is recorded, not run.
                artifact(&root, &target, "common", "common", "normal", false),
            ],
        );
        let output = |name: &str| temp.path().join(name);
        let all = ["alpha".to_owned(), "common".to_owned()];
        write_inventory(&metadata_path, &full, &target, &all, &output("all.json")).unwrap();
        let inventory = read_inventory(&output("all.json")).unwrap();
        assert_eq!(inventory.scope, all);
        assert_eq!(runnable_artifacts(&inventory).unwrap().len(), 2);
        write_inventory(
            &metadata_path,
            &scoped,
            &target,
            &["alpha".to_owned(), "alpha".to_owned()],
            &output("alpha.json"),
        )
        .unwrap();
        let inventory = read_inventory(&output("alpha.json")).unwrap();
        assert_eq!(inventory.scope, ["alpha"]);
        assert_eq!(inventory.workspace_packages, all);
        assert_eq!(runnable_artifacts(&inventory).unwrap().len(), 1);
        for (messages, scope, reason) in [
            (&full, vec!["alpha".to_owned()], "outside the scope"),
            (&scoped, all.to_vec(), "omits runnable target common"),
            (&scoped, vec![], "scope is empty"),
            (&scoped, vec!["zeta".to_owned()], "unknown package zeta"),
        ] {
            let error = write_inventory(
                &metadata_path,
                messages,
                &target,
                &scope,
                &output("refused.json"),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(reason), "{scope:?}: {error}");
        }
        let mut duplicate = inventory.clone();
        duplicate.artifacts.push({
            let mut copy = duplicate.artifacts[0].clone();
            copy.executable = Some("debug/deps/other".to_owned());
            copy
        });
        let error = runnable_artifacts(&duplicate).unwrap_err().to_string();
        assert!(error.contains("ambiguous"), "{error}");
        let mut empty = inventory;
        empty.artifacts.clear();
        assert!(runnable_artifacts(&empty).is_err());
    }

    #[test]
    fn runner_config_binds_partition_host_and_inventory() {
        let workspace = fixture::Workspace::new(Mode::Instrumented);
        let temp = workspace.temp.path();
        let helper = temp.join("delivery helper.exe");
        fs::write(&helper, []).unwrap();
        let diagnostics = temp.join("diagnostics");
        fs::create_dir(&diagnostics).unwrap();
        let partition = PartitionScheme::new(3, 8).unwrap();
        let root = temp.join("root");
        let ledger = temp.join("ledger.jsonl");
        let write = |output: &Path, host: &str, job_started: u64| {
            write_runner_config(&RunnerConfigOptions {
                root: &root,
                host,
                helper: &helper,
                inventory: &workspace.inventory,
                partition: &partition,
                target_dir: &workspace.target,
                ledger: &ledger,
                diagnostics: &diagnostics,
                job_started,
                job_minutes: 45,
                output,
            })
        };
        let config = temp.join("runner.toml");
        let now = unix_now().unwrap();
        write(&config, fixture::HOST, now).unwrap();
        let value: toml::Value = toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        let runner: Vec<_> = value["target"][fixture::HOST]["runner"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        let flag = |name: &str| {
            let position = runner.iter().position(|arg| arg == name).unwrap();
            runner[position + 1].clone()
        };
        assert_eq!(runner[0], helper.to_str().unwrap());
        assert_eq!(runner[1..3], ["coverage", "dispatch"]);
        assert_eq!(flag("--partition"), "3");
        assert_eq!(flag("--partitions"), "8");
        assert_eq!(flag("--host"), fixture::HOST);
        assert_eq!(
            flag("--deadline"),
            shard_deadline(now, 45).unwrap().to_string()
        );
        assert_eq!(runner.last().unwrap(), "--");
        // The configuration is never replaced.
        assert!(write(&config, fixture::HOST, now).is_err());
        for (host, started, reason) in [
            ("x86_64 linux", now, "invalid Cargo host target"),
            (fixture::HOST, now + 3600, "in the future"),
            (
                fixture::HOST,
                now - 45 * 60,
                "passed before its tests started",
            ),
        ] {
            let error = write(&temp.join("other.toml"), host, started)
                .unwrap_err()
                .to_string();
            assert!(error.contains(reason), "{reason}: {error}");
        }
    }

    fn names_of(runs: &[Vec<String>]) -> Vec<String> {
        runs.iter()
            .flat_map(|run| {
                assert_eq!(run[0], "--exact");
                run[1..].iter().cloned()
            })
            .collect()
    }

    #[tokio::test]
    async fn dispatch_lists_every_executable_and_runs_only_assigned_chunks() {
        let workspace = fixture::Workspace::new(Mode::Instrumented);
        let temp = workspace.temp.path();
        let partition = PartitionScheme::new(2, 3).unwrap();
        let ledger = temp.join("ledger.jsonl");
        let diagnostics = temp.join("diagnostics");
        let mut launcher = workspace.launcher();
        launcher.profiles = Some(workspace.target.clone());
        workspace
            .dispatch_all(&mut launcher, &partition, &ledger, &diagnostics, &[])
            .await
            .unwrap();
        let records = plan::read_ledger(&ledger).unwrap();
        assert_eq!(records.len(), workspace.runnable().len());
        let proven =
            plan::validate_run_ledger(&workspace.value, &partition, fixture::HOST, &[], &records)
                .unwrap();
        assert_eq!(
            validate_run_ledger(&workspace.inventory, &partition, fixture::HOST, &ledger).unwrap(),
            proven
        );
        let assigned: Vec<String> = proven
            .executables
            .iter()
            .flat_map(|tests| tests.assigned.iter().cloned())
            .collect();
        // Selections are made in inventory order; compare as sets.
        let mut ran = names_of(&launcher.runs);
        ran.sort();
        let mut expected = assigned.clone();
        expected.sort();
        assert_eq!(ran, expected);
        assert!(!assigned.is_empty());
        for record in &records {
            let (list, chunks) = record.invocations.split_first().unwrap();
            assert_eq!(list.kind, plan::LIST);
            assert_eq!(list.names, record.listed.len());
            assert!(record.started <= record.finished);
            // One list profile plus one per chunk, in this fixture's target.
            assert_eq!(
                record.profraw_after - record.profraw_before,
                1 + chunks.len()
            );
            if record.assigned == 0 {
                assert_eq!(record.action, plan::OMIT);
                assert!(chunks.is_empty());
            } else {
                assert_eq!(record.action, plan::RUN);
                assert_eq!(chunks.len(), 1);
                assert_eq!(chunks[0].announced, Some(record.assigned));
                let log = diagnostics.join(format!(
                    "{}.exact-1.stdout.log",
                    diagnostic_name(&record.executable)
                ));
                assert!(log.is_file(), "{}", log.display());
            }
        }
        // The empty executable is listed and omitted in every partition.
        assert!(records.iter().any(|record| record.listed.is_empty()));
        // Each listing process's profile names its executable by process ID,
        // in the same ledger, without entering the plan.
        let rows = plan::read_spawns(&ledger).unwrap();
        assert_eq!(rows.len(), records.len());
        for (record, row) in records.iter().zip(&rows) {
            assert_eq!(row.role, spawns::TEST_EXECUTABLE);
            assert_eq!(row.test, record.artifact);
            assert_eq!(row.pid as usize, record.profraw_before + 1);
            assert!(
                Path::new(&row.executable).ends_with(&record.executable),
                "{row:?}"
            );
        }
    }

    #[tokio::test]
    async fn every_listed_test_runs_exactly_once_across_partitions() {
        for count in 1..=4 {
            let workspace = fixture::Workspace::new(Mode::Uninstrumented);
            let temp = workspace.temp.path();
            let mut owners = BTreeMap::new();
            let mut listed = BTreeSet::new();
            for index in 1..=count {
                let partition = PartitionScheme::new(index, count).unwrap();
                let ledger = temp.join(format!("ledger-{index}.jsonl"));
                let mut launcher = workspace.launcher();
                workspace
                    .dispatch_all(&mut launcher, &partition, &ledger, &temp.join("d"), &[])
                    .await
                    .unwrap();
                let records = plan::read_ledger(&ledger).unwrap();
                let plan = plan::validate_run_ledger(
                    &workspace.value,
                    &partition,
                    fixture::HOST,
                    &[],
                    &records,
                )
                .unwrap();
                for tests in &plan.executables {
                    for name in &tests.listed {
                        listed.insert((tests.artifact.clone(), name.clone()));
                    }
                    for name in &tests.assigned {
                        assert!(
                            owners
                                .insert((tests.artifact.clone(), name.clone()), index)
                                .is_none(),
                            "{name} ran twice"
                        );
                    }
                }
            }
            assert_eq!(
                owners.keys().cloned().collect::<BTreeSet<_>>(),
                listed,
                "{count}"
            );
        }
    }

    #[tokio::test]
    async fn large_assignments_are_split_into_bounded_chunks() {
        let workspace = fixture::Workspace::new(Mode::Uninstrumented);
        let mut launcher = workspace.launcher();
        let first = workspace
            .target
            .join(workspace.runnable()[0].executable.as_deref().unwrap());
        let names: String = (0..2500)
            .map(|index| format!("module::with::a::long::path::case_{index:05}: test\n"))
            .collect();
        launcher.lists.insert(first, names);
        let partition = PartitionScheme::new(1, 1).unwrap();
        let ledger = workspace.temp.path().join("ledger.jsonl");
        workspace
            .dispatch_all(
                &mut launcher,
                &partition,
                &ledger,
                &workspace.temp.path().join("d"),
                &[],
            )
            .await
            .unwrap();
        let records = plan::read_ledger(&ledger).unwrap();
        let large = records
            .iter()
            .find(|record| record.listed.len() == 2500)
            .unwrap();
        assert!(large.invocations.len() > 3, "{}", large.invocations.len());
        plan::validate_run_ledger(&workspace.value, &partition, fixture::HOST, &[], &records)
            .unwrap();
        for run in &launcher.runs {
            let args: Vec<_> = run.iter().map(String::as_str).collect();
            assert!(
                partition::windows_command_line_units(
                    workspace.target.join("debug/deps/x").to_str().unwrap(),
                    &args
                ) <= partition::COMMAND_LINE_BUDGET
            );
        }
    }

    #[tokio::test]
    async fn a_failing_chunk_still_runs_the_remaining_chunks() {
        let workspace = fixture::Workspace::new(Mode::Uninstrumented);
        let mut launcher = workspace.launcher();
        let first = workspace
            .target
            .join(workspace.runnable()[0].executable.as_deref().unwrap());
        let names: String = (0..2500)
            .map(|index| format!("module::with::a::long::path::case_{index:05}: test\n"))
            .collect();
        launcher.lists.insert(first, names);
        launcher.failing_run = Some(1);
        let partition = PartitionScheme::new(1, 1).unwrap();
        let ledger = workspace.temp.path().join("ledger.jsonl");
        workspace
            .dispatch_all(
                &mut launcher,
                &partition,
                &ledger,
                &workspace.temp.path().join("d"),
                &[],
            )
            .await
            .unwrap();
        let records = plan::read_ledger(&ledger).unwrap();
        let large = records
            .iter()
            .find(|record| record.listed.len() == 2500)
            .unwrap();
        let (_, chunks) = large.invocations.split_first().unwrap();
        assert!(chunks.len() > 2, "{}", chunks.len());
        assert_eq!(chunks[0].status_code, Some(101));
        assert!(chunks[1..].iter().all(|chunk| chunk.status_code == Some(0)));
        assert_eq!(
            chunks.iter().map(|chunk| chunk.names).sum::<usize>(),
            large.assigned
        );
        assert!(!large.success);
        assert_eq!(large.status_code, Some(101));
        assert!(
            plan::validate_run_ledger(&workspace.value, &partition, fixture::HOST, &[], &records)
                .is_err()
        );
    }

    #[tokio::test]
    async fn dispatch_failures_are_recorded_and_fail_closed() {
        type Setup = fn(&mut fixture::ScriptedLauncher);
        let cases: [(Setup, &str, &str); 5] = [
            (
                |launcher| launcher.announce_offset = 1,
                "libtest announced",
                plan::RUN,
            ),
            (
                |launcher| launcher.stall = true,
                "deadline reached while",
                plan::RUN,
            ),
            (
                |launcher| launcher.list_error = true,
                "scripted list failure",
                plan::LIST,
            ),
            (
                |launcher| launcher.run_error = true,
                "scripted run failure",
                plan::RUN,
            ),
            (
                |launcher| {
                    for list in launcher.lists.values_mut() {
                        list.push_str("bench: bench\n");
                    }
                },
                "benchmark",
                plan::LIST,
            ),
        ];
        for (setup, reason, action) in cases {
            let workspace = fixture::Workspace::new(Mode::Uninstrumented);
            let temp = workspace.temp.path();
            let mut launcher = workspace.launcher();
            setup(&mut launcher);
            let partition = PartitionScheme::new(1, 1).unwrap();
            let ledger = temp.join("ledger.jsonl");
            let diagnostics = temp.join("diagnostics");
            let error = format!(
                "{:#}",
                workspace
                    .dispatch_all(&mut launcher, &partition, &ledger, &diagnostics, &[])
                    .await
                    .unwrap_err()
            );
            assert!(error.contains(reason), "{reason}: {error}");
            let records = plan::read_ledger(&ledger).unwrap();
            let last = records.last().unwrap();
            assert!(!last.success, "{reason}");
            assert_eq!(last.action, action, "{reason}");
            let error = plan::validate_run_ledger(
                &workspace.value,
                &partition,
                fixture::HOST,
                &[],
                &records,
            )
            .unwrap_err()
            .to_string();
            assert!(!error.is_empty());
            if reason.starts_with("deadline") {
                let stall = fs::read_dir(&diagnostics)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                    .find(|name| name.ends_with(".exact-1.stall.json"))
                    .unwrap();
                let report: StallReport = read_json(&diagnostics.join(stall)).unwrap();
                assert_eq!(report.unfinished_tests, ["tests::stuck"]);
            }
        }

        // A failing selection is returned to Cargo and recorded as failed.
        let workspace = fixture::Workspace::new(Mode::Uninstrumented);
        let temp = workspace.temp.path();
        let mut launcher = workspace.launcher();
        launcher.status = 101;
        let partition = PartitionScheme::new(1, 1).unwrap();
        let artifact = workspace.runnable()[0];
        let executable = workspace
            .target
            .join(artifact.executable.as_deref().unwrap());
        let ledger = temp.join("ledger.jsonl");
        let diagnostics = temp.join("diagnostics");
        fs::create_dir(&diagnostics).unwrap();
        let current = workspace.root.join(&artifact.package_root);
        let options = |deadline: u64, args: &'static [OsString]| DispatchOptions {
            root: &workspace.root,
            inventory: &workspace.inventory,
            host: fixture::HOST,
            partition: &partition,
            target_dir: &workspace.target,
            ledger: &ledger,
            diagnostics: &diagnostics,
            deadline,
            executable: &executable,
            args,
        };
        let later = unix_now().unwrap() + 3600;
        let status = dispatch_with(&mut launcher, &options(later, &[]), &current, &[])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.code(), Some(101));
        let records = plan::read_ledger(&ledger).unwrap();
        assert_eq!(records[0].status_code, Some(101));
        assert!(!records[0].success);

        // A passed deadline is recorded before anything starts.
        let error = dispatch_with(&mut launcher, &options(1, &[]), &current, &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("passed before starting"), "{error}");
        assert_eq!(
            plan::read_ledger(&ledger).unwrap().last().unwrap().action,
            plan::DEADLINE
        );
        // Cargo-supplied libtest arguments, a foreign directory and an
        // unknown executable are refused without a record.
        static ARGS: [OsString; 0] = [];
        let before = plan::read_ledger(&ledger).unwrap().len();
        let extra = [OsString::from("--nocapture")];
        let mut with_args = options(later, &ARGS);
        with_args.args = &extra;
        let error = dispatch_with(&mut launcher, &with_args, &current, &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("unexpected libtest arguments"), "{error}");
        let error = dispatch_with(
            &mut launcher,
            &options(later, &ARGS),
            &workspace.root.join("elsewhere"),
            &[],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("Cargo ran"), "{error}");
        let stranger = workspace.target.join("debug/deps/stranger");
        fs::write(&stranger, b"").unwrap();
        let mut unknown = options(later, &ARGS);
        unknown.executable = &stranger;
        let error = dispatch_with(&mut launcher, &unknown, &current, &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown test executable"), "{error}");
        assert_eq!(plan::read_ledger(&ledger).unwrap().len(), before);
    }

    #[tokio::test]
    async fn excluded_executables_are_listed_with_their_reason_but_never_run() {
        let workspace = fixture::Workspace::new(Mode::Uninstrumented);
        let temp = workspace.temp.path();
        let artifact = workspace.runnable()[0];
        let key = partition::artifact_key(artifact);
        let reason = "fixture exclusion";
        let exclusions = [(fixture::HOST, key.as_str(), reason)];
        let partition = PartitionScheme::new(1, 1).unwrap();
        let ledger = temp.join("ledger.jsonl");
        let mut launcher = workspace.launcher();
        workspace
            .dispatch_all(
                &mut launcher,
                &partition,
                &ledger,
                &temp.join("d"),
                &exclusions,
            )
            .await
            .unwrap();
        let records = plan::read_ledger(&ledger).unwrap();
        let excluded = records
            .iter()
            .find(|record| record.artifact == key)
            .unwrap();
        assert_eq!(excluded.action, plan::EXCLUDE);
        assert_eq!(excluded.reason.as_deref(), Some(reason));
        assert!(!excluded.listed.is_empty());
        assert_eq!(excluded.invocations.len(), 1);
        let plan = plan::validate_run_ledger(
            &workspace.value,
            &partition,
            fixture::HOST,
            &exclusions,
            &records,
        )
        .unwrap();
        let tests = plan
            .executables
            .iter()
            .find(|tests| tests.artifact == key)
            .unwrap();
        assert_eq!(tests.excluded.as_deref(), Some(reason));
        assert!(tests.assigned.is_empty());
        // Another host's entry does not apply; the record then disagrees.
        let other = [("aarch64-apple-darwin", key.as_str(), reason)];
        let error = plan::validate_run_ledger(
            &workspace.value,
            &partition,
            fixture::HOST,
            &other,
            &records,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("exclusion reason"), "{error}");
        // Stale, reasonless and repeated entries fail.
        for (table, message) in [
            (
                vec![(fixture::HOST, "kuru-memory/lib/gone", reason)],
                "names no test executable",
            ),
            (
                vec![(fixture::HOST, "gone/lib/gone", reason)],
                "names no workspace package",
            ),
            (vec![(fixture::HOST, key.as_str(), " ")], "has no reason"),
            (
                vec![
                    (fixture::HOST, key.as_str(), reason),
                    (fixture::HOST, key.as_str(), reason),
                ],
                "repeated",
            ),
        ] {
            let error = plan::check_exclusions(&table, fixture::HOST, &workspace.value)
                .unwrap_err()
                .to_string();
            assert!(error.contains(message), "{message}: {error}");
        }
        plan::check_exclusions(
            &[("aarch64-apple-darwin", "gone/lib/gone", reason)],
            fixture::HOST,
            &workspace.value,
        )
        .unwrap();
    }

    #[test]
    fn scoped_inventories_skip_exclusions_for_packages_they_do_not_build() {
        let workspace = fixture::Workspace::new(Mode::Uninstrumented);
        let reason = "not built by this scope";
        // A host row for an unbuilt workspace package does not fail a scoped
        // (kuru-memory) inventory, and still leaves nothing excluded there.
        plan::check_exclusions(
            &[(fixture::HOST, "kuru-runtime/lib/kuru_runtime", reason)],
            fixture::HOST,
            &workspace.value,
        )
        .unwrap();
        // In scope it must still name an executable; a non-member always fails.
        for (artifact, message) in [
            ("kuru-memory/lib/gone", "names no test executable"),
            ("gone/lib/gone", "names no workspace package"),
        ] {
            let error = plan::check_exclusions(
                &[(fixture::HOST, artifact, reason)],
                fixture::HOST,
                &workspace.value,
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(message), "{artifact}: {error}");
        }
    }

    #[tokio::test]
    async fn run_ledgers_refuse_every_inconsistency() {
        let workspace = fixture::Workspace::new(Mode::Uninstrumented);
        let temp = workspace.temp.path();
        let partition = PartitionScheme::new(1, 2).unwrap();
        let ledger = temp.join("ledger.jsonl");
        let mut launcher = workspace.launcher();
        workspace
            .dispatch_all(&mut launcher, &partition, &ledger, &temp.join("d"), &[])
            .await
            .unwrap();
        let records = plan::read_ledger(&ledger).unwrap();
        let validate = |records: &[RunnerRecord]| {
            plan::validate_run_ledger(&workspace.value, &partition, fixture::HOST, &[], records)
                .map(drop)
                .map_err(|error| error.to_string())
        };
        validate(&records).unwrap();
        let running = records
            .iter()
            .position(|record| record.action == plan::RUN)
            .unwrap();
        type Tamper = fn(&mut Vec<RunnerRecord>, usize);
        let cases: [(Tamper, &str); 16] = [
            (
                |records, _| records.push(records[0].clone()),
                "extra records",
            ),
            (|records, _| drop(records.pop()), "omits test executables"),
            (
                |records, _| {
                    let copy = records[0].clone();
                    records[1] = copy;
                },
                "more than once",
            ),
            (|records, _| records[0].schema = 3, "schema"),
            (
                |records, _| records[0].args.push("--x".to_owned()),
                "libtest arguments",
            ),
            (
                |records, at| records[at].artifact = "x/lib/x".to_owned(),
                "is recorded as",
            ),
            (
                |records, at| records[at].cwd = "elsewhere".to_owned(),
                "working directory",
            ),
            (|records, at| records[at].listed.reverse(), "unsorted"),
            (
                |records, at| records[at].listed.push("zz\u{7}".to_owned()),
                "invalid test name",
            ),
            (
                |records, at| records[at].list_sha256 = "0".repeat(64),
                "list digest",
            ),
            (
                |records, at| records[at].assigned += 1,
                "assigned tests differ",
            ),
            (
                |records, at| records[at].action = plan::OMIT.to_owned(),
                "wrong runner action",
            ),
            (
                |records, at| records[at].invocations[1].announced = Some(99),
                "announced",
            ),
            (
                |records, at| records[at].invocations[1].names += 1,
                "exceed its assignment",
            ),
            (
                |records, at| drop(records[at].invocations.pop()),
                "ran 0 of",
            ),
            (
                |records, at| records[at].invocations[0].kind = plan::EXACT.to_owned(),
                "expected list",
            ),
        ];
        for (tamper, reason) in cases {
            let mut changed = records.clone();
            tamper(&mut changed, running);
            let error = validate(&changed).unwrap_err();
            assert!(error.contains(reason), "{reason}: {error}");
        }
        let mut backwards = records.clone();
        backwards[running].finished = 0;
        backwards[running].started = 5;
        assert!(
            validate(&backwards)
                .unwrap_err()
                .contains("finished before")
        );
        let mut failed = records;
        failed[running].invocations[1].status_code = Some(1);
        assert!(validate(&failed).unwrap_err().contains("did not succeed"));
    }

    #[test]
    fn profile_summaries_bound_and_digest_profiles() {
        let temp = TempDir::new().unwrap();
        let directory = temp.path();
        let error = profile_summary(directory, Mode::Instrumented)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no nonempty raw profiles"), "{error}");
        assert_eq!(
            profile_summary(directory, Mode::Uninstrumented)
                .unwrap()
                .count,
            0
        );
        fs::write(directory.join("b.profraw"), b"bb").unwrap();
        fs::write(directory.join("a.profraw"), b"").unwrap();
        fs::write(directory.join("other.txt"), b"x").unwrap();
        let summary = profile_summary(directory, Mode::Instrumented).unwrap();
        assert_eq!((summary.count, summary.bytes), (2, 2));
        let manifest = format!(
            "a.profraw 0 {}\nb.profraw 2 {}\n",
            archive::digest(b""),
            archive::digest(b"bb")
        );
        assert_eq!(
            summary.manifest_sha256,
            archive::digest(manifest.as_bytes())
        );
        let error = profile_summary(directory, Mode::Uninstrumented)
            .unwrap_err()
            .to_string();
        assert!(error.contains("produced raw profiles"), "{error}");
        assert_eq!(count_profiles(directory).unwrap(), 2);
        assert_eq!(discard_compile_profiles(directory).unwrap(), 2);
        assert_eq!(count_profiles(directory).unwrap(), 0);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(directory.join("other.txt"), directory.join("l.profraw"))
                .unwrap();
            let error = profile_summary(directory, Mode::Instrumented)
                .unwrap_err()
                .to_string();
            assert!(error.contains("not a regular file"), "{error}");
            assert!(discard_compile_profiles(directory).is_err());
        }
    }

    #[tokio::test]
    async fn evidence_binds_every_file_and_is_written_once() {
        for mode in [Mode::Instrumented, Mode::Uninstrumented] {
            let workspace = fixture::Workspace::new(mode);
            let partition = PartitionScheme::new(1, 2).unwrap();
            let output = workspace.temp.path().join("evidence");
            workspace
                .evidence(mode, &partition, 2, &output)
                .await
                .unwrap();
            let attempt = output.join("attempt-2");
            let mut expected = vec![
                INVENTORY_FILE,
                JOB_LEDGER_FILE,
                PLAN_FILE,
                PROFILE_ENV_FILE,
                RECEIPT_FILE,
                RUNNER_LEDGER_FILE,
            ];
            if mode == Mode::Instrumented {
                expected.extend([LCOV_FILE, LINES_FILE]);
            }
            exact_entries(&attempt, &expected).unwrap();
            let receipt: Receipt = read_versioned(&attempt.join(RECEIPT_FILE), "receipt").unwrap();
            let profile_env: ProfileEnv =
                serde_json::from_slice(&fs::read(attempt.join(PROFILE_ENV_FILE)).unwrap()).unwrap();
            assert_eq!(profile_env, fixture::profile_env());
            assert_eq!(
                receipt.profile_env_sha256,
                digest_json(&profile_env).unwrap()
            );
            assert_eq!(receipt.partition, partition);
            assert_eq!(receipt.run_attempt, "2");
            assert_eq!(receipt.mode, mode);
            assert_eq!(receipt.instrumentation, mode.contract());
            assert_eq!(receipt.lcov.is_some(), mode == Mode::Instrumented);
            assert_eq!(receipt.lines.is_some(), mode == Mode::Instrumented);
            if let Some(lines) = &receipt.lines {
                let bytes = fs::read(attempt.join(LINES_FILE)).unwrap();
                assert_eq!(lines.sha256, archive::digest(&bytes));
                assert_eq!(lines.bytes, bytes.len() as u64);
                assert_eq!((lines.files, lines.instantiations), (1, 1));
                assert_eq!((lines.count, lines.covered), (20, 10));
            }
            assert_eq!(receipt.profiles.count > 0, mode == Mode::Instrumented);
            let plan: plan::PartitionPlan =
                read_versioned(&attempt.join(PLAN_FILE), "plan").unwrap();
            assert_eq!(receipt.plan_sha256, digest_json(&plan).unwrap());
            assert_eq!(receipt.tests_sha256, plan::tests_sha256(&plan).unwrap());
            assert_eq!(
                receipt.runner_ledger_sha256,
                archive::digest(&fs::read(attempt.join(RUNNER_LEDGER_FILE)).unwrap())
            );
            // The same output is never written twice.
            let error = workspace
                .evidence(mode, &partition, 2, &output)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("create"), "{error}");
        }
    }

    #[tokio::test]
    async fn evidence_refuses_mismatched_modes_and_tools() {
        let workspace = fixture::Workspace::new(Mode::Uninstrumented);
        let temp = workspace.temp.path();
        let partition = PartitionScheme::new(1, 1).unwrap();
        let ledger = temp.join("ledger.jsonl");
        let mut launcher = workspace.launcher();
        workspace
            .dispatch_all(&mut launcher, &partition, &ledger, &temp.join("d"), &[])
            .await
            .unwrap();
        let job = temp.join("job.json");
        fs::write(&job, b"{}").unwrap();
        let options = |mode, lcov: Option<&'static Path>| ReceiptOptions {
            root: temp,
            mode,
            partition: &partition,
            inventory: &workspace.inventory,
            ledger: &ledger,
            job_ledger: &job,
            profiles: temp,
            lcov,
            lines: lcov,
            run_attempt: "1",
            expected_source: fixture::SOURCE,
            llvm_cov: None,
            profile_env: Box::leak(Box::new(ProfileEnv::new())),
            output: Box::leak(temp.join("out").into_boxed_path()),
        };
        let error = write_evidence(
            &options(Mode::Uninstrumented, None),
            fixture::identity(Mode::Instrumented),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("tool identity"), "{error}");
        let error = write_evidence(
            &options(Mode::Instrumented, None),
            fixture::identity(Mode::Instrumented),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("whole workspace"), "{error}");
        let error = write_evidence(
            &options(Mode::Uninstrumented, None),
            fixture::identity(Mode::Uninstrumented),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("job ledger"), "{error}");
    }

    #[test]
    fn libtest_announcements_record_the_first_selection_size() {
        for (text, expected) in [
            ("\nrunning 2 tests\ntest a ... ok\n", Some(2)),
            ("running 1 test\n", Some(1)),
            ("running 0 tests\n", Some(0)),
            ("running 3 tests\nrunning 9 tests\n", Some(3)),
            ("running many tests\n", None),
            ("running  tests\n", None),
            ("", None),
        ] {
            let mut progress = LibtestProgress::default();
            progress.observe(text.as_bytes());
            assert_eq!(progress.announced, expected, "{text:?}");
        }
    }

    /// No-op probes that the live libtest test lists and selects exactly.
    #[test]
    fn libtest_probe_alpha() {}

    #[test]
    fn libtest_probe_beta() {}

    const PROBES: [&str; 2] = [
        "coverage::tests::libtest_probe_alpha",
        "coverage::tests::libtest_probe_beta",
    ];

    /// Run this test executable with an exact selection, supervised as the
    /// runner supervises a chunk, without installing runner signal handlers.
    async fn run_self(args: &[String], log: PathBuf) -> Supervision {
        let executable = std::env::current_exe().unwrap();
        #[cfg(unix)]
        {
            let mut command = std::process::Command::new(&executable);
            command
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit());
            supervise_group(
                command,
                tokio::io::sink(),
                log,
                Duration::from_secs(120),
                RUNNER_CLEANUP_TIMEOUT,
                std::future::pending(),
            )
            .await
            .unwrap()
        }
        #[cfg(windows)]
        {
            let artifact = fixture::artifact("kuru-delivery", "lib", "kuru_delivery");
            SystemLauncher
                .run(&Launch {
                    executable: &executable,
                    artifact: &artifact,
                    args,
                    log: log.clone(),
                    remaining: Duration::from_secs(120),
                    ledger: &log,
                })
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn live_libtest_lists_and_runs_exact_selections() {
        let temp = TempDir::new().unwrap();
        let executable = std::env::current_exe().unwrap();
        let listed = partition::parse_libtest_list(
            &SystemLauncher
                .list(&executable, LIST_TIMEOUT)
                .await
                .unwrap(),
        )
        .unwrap();
        for probe in PROBES {
            assert!(listed.binary_search(&probe.to_owned()).is_ok(), "{probe}");
        }
        let mut args = vec!["--exact".to_owned()];
        args.extend(PROBES.map(str::to_owned));
        let supervision = run_self(&args, temp.path().join("exact.log")).await;
        let Supervision::Exited(status, announced) = supervision else {
            panic!("the exact probe selection did not exit");
        };
        assert!(status.success(), "{status:?}");
        assert_eq!(announced, Some(2));

        // A selection sized to the command-line budget still starts: absent
        // names are ordinary filters that select nothing. Short names put the
        // most quoted arguments on the line, so on Windows the platform
        // launcher's own 32,767-unit check sees the budget's worst case.
        let program = executable.to_str().unwrap();
        let mut filler = 0;
        loop {
            let name = format!("absent_{filler:06}");
            let mut next: Vec<&str> = args.iter().map(String::as_str).collect();
            next.push(&name);
            if partition::windows_command_line_units(program, &next)
                > partition::COMMAND_LINE_BUDGET
            {
                break;
            }
            args.push(name);
            filler += 1;
        }
        assert!(filler > 100);
        let supervision = run_self(&args, temp.path().join("budget.log")).await;
        let Supervision::Exited(status, announced) = supervision else {
            panic!("the budget-sized selection did not exit");
        };
        assert!(status.success(), "{status:?}");
        assert_eq!(announced, Some(2));
        let error = SystemLauncher
            .list(&temp.path().join("missing"), LIST_TIMEOUT)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("list the tests"), "{error:#}");
    }

    /// The group settle never outlives its wait's deadline.
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_group_settle_is_bounded_by_the_wait_deadline() {
        let mut command = group_command("sleep 30 & exit 0");
        command.stdout(std::process::Stdio::null());
        let mut child = GroupProcess::spawn(command, Duration::from_secs(60)).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let started = std::time::Instant::now();
        match child.wait(Duration::ZERO).await {
            Ok(status) => assert!(status.success()),
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error}"),
        }
        assert!(started.elapsed() < Duration::from_secs(10));
        let status = child.wait(GROUP_BOUND).await.unwrap();
        assert!(status.success());
        assert_eq!(child.presence_after_reap().as_deref(), Some("Absent"));
    }

    async fn git(root: &Path, args: &[&str]) -> std::process::Output {
        let mut command = crate::command::rooted(root, "git");
        command.args(["-c", "commit.gpgSign=false"]).args(args);
        let started = std::time::Instant::now();
        crate::command::bounded_output(&mut command, std::time::Duration::from_secs(30), 64 * 1024)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "git {args:?} in {root:?} failed after {:?}: {error}",
                    started.elapsed()
                )
            })
    }

    // A missing working directory fails the launch at once, so the helper's
    // failure text is checked without waiting for its 30-second deadline.
    #[tokio::test]
    async fn git_helper_names_arguments_directory_and_elapsed_time_when_the_launch_fails() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("absent");
        let panic = tokio::spawn({
            let missing = missing.clone();
            async move { git(&missing, &["rev-parse", "HEAD"]).await }
        })
        .await
        .unwrap_err()
        .into_panic();
        let message = panic.downcast_ref::<String>().cloned().unwrap();
        for required in [
            r#"git ["rev-parse", "HEAD"]"#,
            &format!("in {missing:?} failed after "),
        ] {
            assert!(message.contains(required), "missing {required}: {message}");
        }
    }

    #[tokio::test]
    async fn modified_tracked_source_cannot_claim_head_identity() {
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
        let error = identity(root, head.trim(), Some(Path::new("missing-llvm-cov")))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("modified tracked files"));
        let error = identity(root, "0000", None).await.unwrap_err();
        assert!(error.to_string().contains("expected 0000"), "{error}");
        assert!(
            git(root, &["checkout", "--", "source.rs"])
                .await
                .status
                .success()
        );
        let observed = identity(root, head.trim(), None).await.unwrap();
        assert_eq!(observed.source, head.trim());
        assert_eq!(observed.cargo_llvm_cov, None);
        assert!(
            observed
                .rustc
                .contains(&format!("host: {}", observed.target))
        );
        assert_eq!(observed.target_os, std::env::consts::OS);
        verify_source(root, head.trim(), None).await.unwrap();
    }
}
