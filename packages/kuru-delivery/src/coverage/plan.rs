//! Runner ledger records and the partition plan derived from them.
//!
//! Cargo invokes the task-private runner once per test executable. The runner
//! lists the executable's tests, runs this partition's share with explicit
//! exact selections and appends one [`RunnerRecord`]. [`validate_run_ledger`]
//! recomputes every assignment from the recorded list and returns the
//! [`PartitionPlan`] the receipt binds; the merge validates each uploaded
//! ledger again and requires the plans of one OS to be disjoint and complete.

use super::{
    Inventory, RUNNER_LEDGER_LIMIT, SCHEMA, digest_json,
    partition::{self, PartitionScheme},
    read_bounded, runnable_artifacts,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

/// Ledger action of an executable that ran this partition's tests.
pub const RUN: &str = "run";
/// Ledger action of an executable with no tests assigned to this partition.
pub const OMIT: &str = "omit";
/// Ledger action of an executable excluded on this host with a reason.
pub const EXCLUDE: &str = "exclude";
/// Ledger action of an executable the deadline stopped.
pub const DEADLINE: &str = "deadline";
/// Invocation kinds.
pub const LIST: &str = "list";
pub const EXACT: &str = "exact";

/// One process the runner started for an executable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationRecord {
    /// `list` or `exact`.
    pub kind: String,
    /// Names listed (`list`) or selected (`exact`).
    pub names: usize,
    /// Canonical digest of those names.
    pub names_sha256: String,
    /// Libtest's `running N tests` count, for exact selections.
    pub announced: Option<usize>,
    pub started: u64,
    pub finished: u64,
    pub status_code: Option<i32>,
}

/// The runner's record of one Cargo-invoked test executable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerRecord {
    pub schema: u32,
    /// Target-relative executable path, as the inventory records it.
    pub executable: String,
    /// [`partition::artifact_key`] of the executable.
    pub artifact: String,
    pub action: String,
    pub cwd: String,
    /// Cargo-supplied libtest arguments; always empty.
    pub args: Vec<String>,
    /// Every test the executable listed, sorted.
    pub listed: Vec<String>,
    pub list_sha256: String,
    /// Number and digest of this partition's names.
    pub assigned: usize,
    pub assigned_sha256: String,
    /// The exclusion reason, for an `exclude` action.
    pub reason: Option<String>,
    /// The list run first, then each exact chunk.
    pub invocations: Vec<InvocationRecord>,
    pub started: u64,
    pub finished: u64,
    /// Raw profiles in the target root before and after this executable.
    pub profraw_before: usize,
    pub profraw_after: usize,
    pub success: bool,
    pub status_code: Option<i32>,
}

/// One executable's listed and assigned names in one partition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableTests {
    pub artifact: String,
    pub executable: String,
    pub listed: Vec<String>,
    pub list_sha256: String,
    pub assigned: Vec<String>,
    pub assigned_sha256: String,
    pub excluded: Option<String>,
}

/// The stored, auditable test lists of one partition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionPlan {
    pub schema: u32,
    pub inventory_sha256: String,
    pub partition: PartitionScheme,
    /// Sorted by artifact key.
    pub executables: Vec<ExecutableTests>,
}

/// The exclusion reason of an artifact on a host, if the table names it.
pub fn excluded_reason(exclusions: &[(&str, &str, &str)], host: &str, key: &str) -> Option<String> {
    exclusions
        .iter()
        .find(|(table_host, artifact, _)| *table_host == host && *artifact == key)
        .map(|(_, _, reason)| (*reason).to_owned())
}

/// Every exclusion for this host must name one inventory test executable
/// once, with a reason. A stale entry fails. An exclusion for a workspace
/// package outside a scoped (uninstrumented) inventory is not built there, so
/// it only has to name a workspace package.
pub fn check_exclusions(
    exclusions: &[(&str, &str, &str)],
    host: &str,
    inventory: &Inventory,
) -> Result<()> {
    let keys: Vec<_> = runnable_artifacts(inventory)?
        .iter()
        .map(partition::artifact_key)
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for (table_host, artifact, reason) in exclusions {
        ensure!(
            seen.insert((*table_host, *artifact)),
            "coverage exclusion {artifact} on {table_host} is repeated"
        );
        if *table_host != host {
            continue;
        }
        ensure!(
            !reason.trim().is_empty(),
            "coverage exclusion {artifact} on {host} has no reason"
        );
        let package = artifact.split('/').next().unwrap_or_default();
        if !inventory.scope.iter().any(|scoped| scoped == package) {
            ensure!(
                inventory
                    .workspace_packages
                    .iter()
                    .any(|member| member == package),
                "coverage exclusion {artifact} on {host} names no workspace package"
            );
            continue;
        }
        ensure!(
            keys.iter().any(|key| key == artifact),
            "coverage exclusion {artifact} on {host} names no test executable of the inventory"
        );
    }
    Ok(())
}

/// Read a runner ledger, refusing schema-1 records by name.
pub fn read_ledger(path: &Path) -> Result<Vec<RunnerRecord>> {
    let bytes = read_bounded(path, RUNNER_LEDGER_LIMIT)?;
    let text = std::str::from_utf8(&bytes).context("coverage runner ledger is not UTF-8")?;
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let value: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("parse runner ledger line {}", index + 1))?;
        match value.get("schema").and_then(serde_json::Value::as_u64) {
            Some(1) => bail!(
                "runner ledger line {} uses coverage schema 1 (package shards)",
                index + 1
            ),
            Some(schema) if schema == u64::from(SCHEMA) => {}
            other => bail!(
                "runner ledger line {} has unsupported schema {other:?}",
                index + 1
            ),
        }
        records.push(
            serde_json::from_value(value)
                .with_context(|| format!("parse runner ledger line {}", index + 1))?,
        );
    }
    Ok(records)
}

fn check_listed(record: &RunnerRecord) -> Result<()> {
    let executable = &record.executable;
    ensure!(
        record.listed.windows(2).all(|pair| pair[0] < pair[1]),
        "{executable} records an unsorted or repeated test list"
    );
    ensure!(
        record.listed.iter().all(|name| !name.is_empty()
            && name.trim() == name
            && !name.chars().any(char::is_control)),
        "{executable} records an invalid test name"
    );
    ensure!(
        record.list_sha256 == partition::list_sha256(&record.listed),
        "{executable} test list digest differs from its names"
    );
    Ok(())
}

fn check_invocation(
    executable: &str,
    invocation: &InvocationRecord,
    kind: &str,
    names: &[String],
) -> Result<()> {
    ensure!(
        invocation.kind == kind,
        "{executable} invocation is {}, expected {kind}",
        invocation.kind
    );
    ensure!(
        invocation.names == names.len() && invocation.names_sha256 == partition::list_sha256(names),
        "{executable} {kind} invocation names differ from the plan"
    );
    ensure!(
        invocation.status_code == Some(0),
        "{executable} {kind} invocation did not succeed"
    );
    ensure!(
        invocation.started <= invocation.finished,
        "{executable} {kind} invocation finished before it started"
    );
    if kind == EXACT {
        ensure!(
            invocation.announced == Some(names.len()),
            "{executable} libtest announced {:?} tests for a {}-name selection",
            invocation.announced,
            names.len()
        );
    }
    Ok(())
}

/// Require Cargo to have invoked every test executable once and the runner to
/// have run exactly this partition's assigned tests of each, successfully.
/// Returns the plan the records prove.
pub fn validate_run_ledger(
    inventory: &Inventory,
    partition: &PartitionScheme,
    host: &str,
    exclusions: &[(&str, &str, &str)],
    records: &[RunnerRecord],
) -> Result<PartitionPlan> {
    partition.validate()?;
    let artifacts = runnable_artifacts(inventory)?;
    check_exclusions(exclusions, host, inventory)?;
    ensure!(
        records.len() <= artifacts.len(),
        "Cargo runner ledger has extra records"
    );
    let mut by_executable = BTreeMap::new();
    for record in records {
        ensure!(record.schema == SCHEMA, "unsupported runner ledger schema");
        ensure!(
            record.args.is_empty(),
            "Cargo supplied unexpected libtest arguments"
        );
        ensure!(
            by_executable
                .insert(record.executable.as_str(), record)
                .is_none(),
            "Cargo invoked a test executable more than once"
        );
    }
    ensure!(
        by_executable.len() == artifacts.len(),
        "Cargo runner ledger omits test executables"
    );
    let mut executables = Vec::new();
    for artifact in &artifacts {
        let executable = artifact
            .executable
            .as_deref()
            .context("runnable artifact has no executable")?;
        let record = by_executable
            .get(executable)
            .with_context(|| format!("Cargo did not invoke {executable}"))?;
        let key = partition::artifact_key(artifact);
        ensure!(
            record.artifact == key,
            "{executable} is recorded as {}, expected {key}",
            record.artifact
        );
        ensure!(
            record.cwd == artifact.package_root,
            "wrong Cargo working directory for {executable}"
        );
        ensure!(
            record.success && record.status_code == Some(0),
            "Cargo runner did not complete {executable} successfully ({})",
            record.action
        );
        ensure!(
            record.started <= record.finished,
            "{executable} finished before it started"
        );
        check_listed(record)?;
        let excluded = excluded_reason(exclusions, host, &key);
        let assigned = if excluded.is_some() {
            Vec::new()
        } else {
            partition.assigned(&key, &record.listed)
        };
        ensure!(
            record.reason == excluded,
            "{executable} exclusion reason {:?} differs from the table's {excluded:?}",
            record.reason
        );
        ensure!(
            record.assigned == assigned.len()
                && record.assigned_sha256 == partition::list_sha256(&assigned),
            "{executable} assigned tests differ from partition {} of {}",
            partition.index,
            partition.count
        );
        let expected = if excluded.is_some() {
            EXCLUDE
        } else if assigned.is_empty() {
            OMIT
        } else {
            RUN
        };
        ensure!(
            record.action == expected,
            "wrong runner action {} for {executable}, expected {expected}",
            record.action
        );
        let (list, chunks) = record
            .invocations
            .split_first()
            .with_context(|| format!("{executable} has no list invocation"))?;
        check_invocation(executable, list, LIST, &record.listed)?;
        // Consecutive chunks of the assigned order prove a disjoint union.
        let mut offset = 0_usize;
        for chunk in chunks {
            let end = offset
                .checked_add(chunk.names)
                .filter(|end| *end <= assigned.len() && chunk.names > 0)
                .with_context(|| format!("{executable} exact chunks exceed its assignment"))?;
            check_invocation(executable, chunk, EXACT, &assigned[offset..end])?;
            offset = end;
        }
        ensure!(
            offset == assigned.len(),
            "{executable} ran {offset} of its {} assigned tests",
            assigned.len()
        );
        executables.push(ExecutableTests {
            artifact: key,
            executable: executable.to_owned(),
            listed: record.listed.clone(),
            list_sha256: record.list_sha256.clone(),
            assigned_sha256: record.assigned_sha256.clone(),
            assigned,
            excluded,
        });
    }
    executables.sort_by(|left, right| left.artifact.cmp(&right.artifact));
    Ok(PartitionPlan {
        schema: SCHEMA,
        inventory_sha256: digest_json(inventory)?,
        partition: partition.clone(),
        executables,
    })
}

/// Digest of what every partition of one OS must share: each executable's
/// artifact, listed-name digest and exclusion.
pub fn tests_sha256(plan: &PartitionPlan) -> Result<String> {
    let shared: Vec<_> = plan
        .executables
        .iter()
        .map(|tests| (&tests.artifact, &tests.list_sha256, &tests.excluded))
        .collect();
    digest_json(&shared)
}
