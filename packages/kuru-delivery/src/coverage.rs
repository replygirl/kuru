//! Fail-closed manifests for sharded native coverage.

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
#[cfg(windows)]
const RUNNER_PROCESS_TIMEOUT: Duration = Duration::from_secs(90 * 60);
#[cfg(windows)]
const RUNNER_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);

pub const SHARDS: [(&str, &[&str]); 4] = [
    ("delivery-archive", &["kuru-delivery", "kuru-archive"]),
    ("application", &["kuru"]),
    ("memory-runtime", &["kuru-memory", "kuru-runtime"]),
    (
        "connectors-core-platform",
        &["kuru-connectors", "kuru-core", "kuru-platform"],
    ),
];

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
    pub output: &'a Path,
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
    ]
    .map(|path| {
        path.to_str()
            .context("Cargo runner configuration path is not UTF-8")
    });
    let [helper, root, target_dir, inventory, selection, ledger] = strings;
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

pub async fn dispatch_test(
    root: &Path,
    inventory_path: &Path,
    selection_path: &Path,
    target_dir: &Path,
    ledger: &Path,
    executable: &Path,
    args: &[OsString],
) -> Result<Option<ExitStatus>> {
    ensure!(root.is_absolute(), "coverage root must be absolute");
    ensure!(
        inventory_path.is_absolute() && selection_path.is_absolute(),
        "coverage manifests must be absolute"
    );
    ensure!(target_dir.is_absolute(), "coverage target must be absolute");
    ensure!(ledger.is_absolute(), "coverage ledger must be absolute");
    let executable_path = if executable.is_absolute() {
        executable.to_owned()
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
    #[cfg(windows)]
    let status = {
        use kuru_platform::windows::process::{NativeSpawnSpec, StandardStream, inherited_stdio};

        let mut spec = NativeSpawnSpec::new(executable_path, std::env::current_dir()?);
        spec.args = args.iter().map(OsString::from).collect();
        spec.environment = std::env::vars_os().collect();
        spec.stdin = inherited_stdio(StandardStream::Input)?;
        spec.stdout = inherited_stdio(StandardStream::Output)?;
        spec.stderr = inherited_stdio(StandardStream::Error)?;
        let mut child = spec.spawn().await?;
        match child.wait(RUNNER_PROCESS_TIMEOUT).await {
            Ok(status) => status,
            Err(error) => {
                let termination = child.terminate();
                let cleanup = child.wait(RUNNER_CLEANUP_TIMEOUT).await;
                bail!(
                    "Cargo test process did not settle: {error}; termination={termination:?}; cleanup={cleanup:?}"
                );
            }
        }
    };
    #[cfg(not(windows))]
    let status = std::process::Command::new(executable_path)
        .args(&args)
        .status()?;
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
    let mut profiles = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.path().extension() == Some(OsStr::new("profraw")) {
            profiles.push(entry.path());
        }
    }
    profiles.sort();
    ensure!(
        !profiles.is_empty(),
        "coverage shard produced no raw profiles"
    );
    ensure!(
        profiles.len() <= PROFILE_COUNT_LIMIT,
        "coverage shard produced too many profiles"
    );
    let total = profiles.iter().try_fold(0_u64, |total, path| {
        total
            .checked_add(fs::symlink_metadata(path)?.len())
            .context("profile total size overflow")
    })?;
    ensure!(
        total <= PROFILE_TOTAL_LIMIT,
        "coverage shard profiles exceed total size limit"
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
    ensure!(
        !run_attempt.is_empty() && run_attempt.bytes().all(|byte| byte.is_ascii_digit()),
        "coverage run attempt must contain only digits"
    );
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

pub async fn collect_profiles(
    root: &Path,
    inventory_path: &Path,
    inputs: &Path,
    target_dir: &Path,
    expected_source: &str,
    run_attempt: &str,
    llvm_cov: &Path,
) -> Result<()> {
    ensure!(
        !run_attempt.is_empty() && run_attempt.bytes().all(|byte| byte.is_ascii_digit()),
        "coverage run attempt must contain only digits"
    );
    let inventory: Inventory = read_json(inventory_path)?;
    ensure!(inventory.schema == SCHEMA, "unsupported inventory schema");
    let inventory_sha256 = digest_json(&inventory)?;
    let observed = identity(root, expected_source, llvm_cov).await?;
    collect_profiles_with_identity(
        &inventory,
        &inventory_sha256,
        inputs,
        target_dir,
        run_attempt,
        &observed,
    )
}

fn collect_profiles_with_identity(
    inventory: &Inventory,
    inventory_sha256: &str,
    inputs: &Path,
    target_dir: &Path,
    run_attempt: &str,
    observed: &ReceiptIdentity,
) -> Result<()> {
    ensure!(
        fs::symlink_metadata(inputs)?.file_type().is_dir(),
        "coverage inputs path is not a directory"
    );
    ensure!(
        fs::symlink_metadata(target_dir)?.file_type().is_dir(),
        "aggregate target is not a directory"
    );
    let mut directories: Vec<_> = fs::read_dir(inputs)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    directories.sort();
    ensure!(
        directories.len() == SHARDS.len(),
        "expected {} shard artifacts",
        SHARDS.len()
    );
    ensure_no_profiles(target_dir)?;

    let mut seen = BTreeSet::new();
    let mut assigned = BTreeSet::new();
    let mut accepted = Vec::new();
    let mut aggregate_count = 0_usize;
    let mut aggregate_bytes = 0_u64;
    for directory in directories {
        ensure!(
            fs::symlink_metadata(&directory)?.file_type().is_dir(),
            "shard artifact is not a directory"
        );
        exact_entries(
            &directory,
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
            receipt.run_attempt == run_attempt,
            "shard run attempt differs"
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
    Ok(())
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
            for (shard, _) in SHARDS {
                let directory = inputs.join(shard);
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
                        run_attempt: "2".to_owned(),
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
            Self {
                _temp: temp,
                inputs,
                target,
                inventory,
                inventory_sha256,
                identity,
            }
        }

        fn shard(&self, name: &str) -> PathBuf {
            self.inputs.join(name)
        }

        fn collect(&self) -> Result<()> {
            collect_profiles_with_identity(
                &self.inventory,
                &self.inventory_sha256,
                &self.inputs,
                &self.target,
                "2",
                &self.identity,
            )
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
        assert!(missing.collect().is_err());
        assert_eq!(fs::read_dir(&missing.target).unwrap().count(), 0);

        let duplicate = AggregateFixture::new();
        let receipt_path = duplicate.shard("memory-runtime").join("receipt.json");
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
                .shard("memory-runtime")
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
        let config = temp.path().join("runner.toml");
        write_runner_config(&RunnerConfigOptions {
            root: &root,
            host: "x86_64-pc-windows-msvc",
            helper: &helper,
            inventory: &inventory_path,
            selection: &selection_path,
            target_dir: &target,
            ledger: &ledger,
            output: &config,
        })
        .unwrap();
        let config: toml::Value = toml::from_str(&fs::read_to_string(config).unwrap()).unwrap();
        let runner = config["target"]["x86_64-pc-windows-msvc"]["runner"]
            .as_array()
            .unwrap();
        assert_eq!(runner[0].as_str(), helper.to_str());
        assert_eq!(runner.last().unwrap().as_str(), Some("--"));

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
