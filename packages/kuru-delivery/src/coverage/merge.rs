//! The receipt-agreement merge of one OS's partitions.
//!
//! The merge does not rebuild the inventory. N independently built partitions
//! must agree instead: every index is present, every receipt is identical on
//! source, tree, lockfile, toolchain, profile environment, mode and inventory,
//! and consistent with the OS label and the expected commit; every uploaded
//! file matches its receipt digest; every runner ledger proves its plan; and
//! each executable's plans are disjoint and complete against its listed
//! tests. Only then does an instrumented merge union the partitions' LCOV and
//! enforce the OS's 91% unique-line gate. Any failure leaves no report file.

use super::{
    ATTEMPT_DIRECTORY, EXCLUDED_ARTIFACTS, INVENTORY_FILE, JOB_LEDGER_FILE, JSON_LIMIT, LCOV_FILE,
    LLVM_COV_VERSION, LOCAL_OS, Mode, PLAN_FILE, PROFILE_COUNT_LIMIT, PROFILE_TOTAL_LIMIT,
    RECEIPT_FILE, RUNNER_LEDGER_FILE, RUNNER_LEDGER_LIMIT, Receipt, SCHEMA, WORKSPACE_PACKAGES,
    archive, canonical_attempt, digest_json, exact_entries,
    lcov::{self, Lcov, Totals},
    ledger::JobLedger,
    os_target,
    partition::PartitionScheme,
    plan::{self, PartitionPlan},
    read_bounded, read_inventory, read_versioned,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct MergeOptions<'a> {
    /// The download directory of this OS's partition evidence.
    pub inputs: &'a Path,
    /// The merged LCOV path; `merge-summary.json` is written beside it.
    pub report: &'a Path,
    pub os: &'a str,
    pub source: &'a str,
    /// The current workflow run attempt; no partition may claim a later one.
    pub attempt: &'a str,
    pub count: u32,
    pub mode: Mode,
    /// The package scope every partition must have built: the whole
    /// workspace when instrumented, the configured packages otherwise.
    pub scope: &'a [String],
}

/// One accepted partition's row of the merge summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionRow {
    pub index: u32,
    pub attempt: u64,
    pub helper_cache: String,
    pub seed_cache: String,
    pub compile_seconds: Option<u64>,
    pub tests_seconds: Option<u64>,
    pub profiles: usize,
    pub dependency_units_rebuilt: u64,
    pub slowest_executable: Option<(String, u64)>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MergeSummary {
    pub schema: u32,
    pub os: String,
    pub mode: Mode,
    pub source: String,
    pub executables: usize,
    pub tests: usize,
    pub excluded: Vec<(String, String)>,
    pub partitions: Vec<PartitionRow>,
    pub coverage: Option<Totals>,
}

pub const SUMMARY_FILE: &str = "merge-summary.json";

/// Accepted evidence of one partition.
struct Accepted {
    index: u32,
    attempt: u64,
    receipt: Receipt,
    plan: PartitionPlan,
    job: JobLedger,
    lcov: Option<Lcov>,
}

/// Parse `<prefix>-coverage-<os>-partition-<k>-attempt-<n>`.
fn artifact_name<'a>(name: &'a str, os: &str) -> Result<(&'a str, u32, u64)> {
    let parsed = (|| {
        let (rest, attempt) = name.rsplit_once("-attempt-")?;
        let (rest, index) = rest.rsplit_once("-partition-")?;
        let prefix = rest.strip_suffix(&format!("-coverage-{os}"))?;
        let index = u32::try_from(canonical_attempt(index)?).ok()?;
        (!prefix.is_empty()).then_some((prefix, index, canonical_attempt(attempt)?))
    })();
    parsed.with_context(|| format!("unexpected coverage evidence artifact {name} for {os}"))
}

/// Sorted names and paths of a directory's entries, which must be directories.
fn directories(directory: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("read {}", directory.display()))?
        .map(|entry| {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("coverage evidence has a non-UTF-8 name"))?;
            ensure!(
                fs::symlink_metadata(entry.path())?.file_type().is_dir(),
                "coverage evidence entry {name} is not a directory"
            );
            Ok((name, entry.path()))
        })
        .collect::<Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}

fn attempt_of(name: &str) -> Result<u64> {
    name.strip_prefix(ATTEMPT_DIRECTORY)
        .and_then(canonical_attempt)
        .with_context(|| format!("coverage evidence {name} is not a canonical attempt directory"))
}

/// Choose each partition's highest uploaded attempt no later than the run.
///
/// download-artifact extracts a pattern matching several artifacts into one
/// directory per artifact name (`<name>/attempt-<n>`), and a pattern matching
/// one artifact directly (`attempt-<n>`), which is valid only for one
/// partition. Failed partitions upload only diagnostics, whose names the
/// evidence pattern cannot match, so the highest attempt is the latest success.
fn latest_attempts(
    inputs: &Path,
    os: &str,
    count: u32,
    max_attempt: u64,
) -> Result<BTreeMap<u32, (u64, PathBuf)>> {
    ensure!(
        fs::symlink_metadata(inputs)
            .with_context(|| format!("coverage inputs {} are missing", inputs.display()))?
            .file_type()
            .is_dir(),
        "coverage inputs path is not a directory"
    );
    let entries = directories(inputs)?;
    let flat = entries
        .iter()
        .filter(|(name, _)| name.starts_with(ATTEMPT_DIRECTORY))
        .count();
    let mut found: BTreeMap<u32, BTreeMap<u64, PathBuf>> = BTreeMap::new();
    if flat > 0 {
        ensure!(
            flat == entries.len(),
            "coverage inputs mix single- and multi-artifact download layouts"
        );
        ensure!(
            count == 1 && entries.len() == 1,
            "a flat download layout holds one partition's single attempt"
        );
        let (name, path) = &entries[0];
        found
            .entry(1)
            .or_default()
            .insert(attempt_of(name)?, path.clone());
    } else {
        let mut prefix = None;
        for (name, path) in &entries {
            let (artifact_prefix, index, attempt) = artifact_name(name, os)?;
            ensure!(
                *prefix.get_or_insert(artifact_prefix) == artifact_prefix,
                "coverage evidence artifacts use different prefixes"
            );
            ensure!(
                (1..=count).contains(&index),
                "coverage evidence {name} names partition {index} outside 1..={count}"
            );
            let inner = directories(path)?;
            let [(inner_name, inner_path)] = &inner[..] else {
                bail!("coverage evidence {name} must hold one attempt directory");
            };
            ensure!(
                attempt_of(inner_name)? == attempt,
                "coverage evidence {name} holds {inner_name}"
            );
            ensure!(
                found
                    .entry(index)
                    .or_default()
                    .insert(attempt, inner_path.clone())
                    .is_none(),
                "coverage partition {index} has duplicate attempt {attempt}"
            );
        }
    }
    let mut latest = BTreeMap::new();
    for index in 1..=count {
        let mut attempts = found
            .remove(&index)
            .with_context(|| format!("coverage partition {index} of {count} has no evidence"))?;
        let (attempt, path) = attempts.pop_last().expect("nonempty attempts");
        ensure!(
            attempt <= max_attempt,
            "coverage partition {index} attempt {attempt} is newer than run attempt {max_attempt}"
        );
        latest.insert(index, (attempt, path));
    }
    Ok(latest)
}

/// The Rust target OS name of a host target.
fn target_os(target: &str) -> Option<&'static str> {
    if target.contains("-linux-") {
        Some("linux")
    } else if target.ends_with("-apple-darwin") {
        Some("macos")
    } else if target.contains("-windows-") {
        Some("windows")
    } else {
        None
    }
}

fn json(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// Receipt fields every partition of one OS must share, by name.
fn agreement(receipt: &Receipt) -> Result<Vec<(&'static str, String)>> {
    Ok(vec![
        ("schema", json(&receipt.schema)?),
        ("scheme", json(&receipt.partition.scheme)?),
        ("partition count", json(&receipt.partition.count)?),
        ("mode", json(&receipt.mode)?),
        ("scope", json(&receipt.scope)?),
        ("source", json(&receipt.source)?),
        ("tree", json(&receipt.tree)?),
        ("Cargo.lock", json(&receipt.cargo_lock_sha256)?),
        ("rustc", json(&receipt.rustc)?),
        ("cargo", json(&receipt.cargo)?),
        ("target", json(&receipt.target)?),
        ("cargo-llvm-cov", json(&receipt.cargo_llvm_cov)?),
        ("instrumentation", json(&receipt.instrumentation)?),
        ("target OS", json(&receipt.target_os)?),
        ("profile environment", json(&receipt.profile_env_sha256)?),
        ("inventory", json(&receipt.inventory_sha256)?),
        ("tests", json(&receipt.tests_sha256)?),
    ])
}

/// Check one partition's evidence against its receipt and the expectation.
fn accept(
    options: &MergeOptions<'_>,
    index: u32,
    attempt: u64,
    directory: &Path,
) -> Result<Accepted> {
    let label = format!("coverage partition {index} (attempt {attempt})");
    let mut expected = vec![
        INVENTORY_FILE,
        JOB_LEDGER_FILE,
        PLAN_FILE,
        RECEIPT_FILE,
        RUNNER_LEDGER_FILE,
    ];
    if options.mode == Mode::Instrumented {
        expected.push(LCOV_FILE);
    }
    exact_entries(directory, &expected).with_context(|| label.clone())?;
    let receipt: Receipt = read_versioned(&directory.join(RECEIPT_FILE), "coverage receipt")
        .with_context(|| label.clone())?;
    let scheme = PartitionScheme::new(index, options.count)?;
    ensure!(
        receipt.partition == scheme,
        "{label} carries a receipt for partition {} of {}",
        receipt.partition.index,
        receipt.partition.count
    );
    ensure!(
        receipt.run_attempt == attempt.to_string(),
        "{label} receipt names attempt {}",
        receipt.run_attempt
    );
    ensure!(
        receipt.mode == options.mode,
        "{label} is {}, expected {}",
        receipt.mode.name(),
        options.mode.name()
    );
    ensure!(
        receipt.source == options.source,
        "{label} was built from {}, expected {}",
        receipt.source,
        options.source
    );
    ensure!(
        receipt.instrumentation == options.mode.contract(),
        "{label} instrumentation contract differs"
    );
    let tool = match options.mode {
        Mode::Instrumented => Some(LLVM_COV_VERSION.to_owned()),
        Mode::Uninstrumented => None,
    };
    ensure!(
        receipt.cargo_llvm_cov == tool,
        "{label} coverage tool {:?} differs from {tool:?}",
        receipt.cargo_llvm_cov
    );
    if options.os != LOCAL_OS {
        let target = os_target(options.os)
            .with_context(|| format!("no host target is declared for {}", options.os))?;
        ensure!(
            receipt.target == target,
            "{label} ran on {}, but {} partitions run on {target}",
            receipt.target,
            options.os
        );
    }
    ensure!(
        target_os(&receipt.target) == Some(receipt.target_os.as_str()),
        "{label} target {} and OS {} disagree",
        receipt.target,
        receipt.target_os
    );
    if options.mode == Mode::Instrumented {
        ensure!(
            receipt.scope == WORKSPACE_PACKAGES,
            "{label} instrumented scope is not the whole workspace"
        );
    }
    ensure!(
        receipt.scope == options.scope,
        "{label} scope {:?} differs from the expected {:?}",
        receipt.scope,
        options.scope
    );

    let inventory =
        read_inventory(&directory.join(INVENTORY_FILE)).with_context(|| label.clone())?;
    ensure!(
        digest_json(&inventory)? == receipt.inventory_sha256,
        "{label} uploaded inventory differs from its receipt"
    );
    ensure!(
        inventory.scope == receipt.scope,
        "{label} inventory scope differs from its receipt"
    );
    let ledger_path = directory.join(RUNNER_LEDGER_FILE);
    ensure!(
        archive::digest(&read_bounded(&ledger_path, RUNNER_LEDGER_LIMIT)?)
            == receipt.runner_ledger_sha256,
        "{label} uploaded runner ledger differs from its receipt"
    );
    let records = plan::read_ledger(&ledger_path).with_context(|| label.clone())?;
    let proven = plan::validate_run_ledger(
        &inventory,
        &scheme,
        &receipt.target,
        &EXCLUDED_ARTIFACTS,
        &records,
    )
    .with_context(|| format!("{label} runner ledger"))?;
    let uploaded: PartitionPlan = read_versioned(&directory.join(PLAN_FILE), "partition plan")
        .with_context(|| label.clone())?;
    ensure!(
        uploaded == proven,
        "{label} uploaded plan differs from its runner ledger"
    );
    ensure!(
        digest_json(&uploaded)? == receipt.plan_sha256
            && plan::tests_sha256(&uploaded)? == receipt.tests_sha256,
        "{label} uploaded plan differs from its receipt"
    );
    let job_bytes = read_bounded(&directory.join(JOB_LEDGER_FILE), JSON_LIMIT)?;
    ensure!(
        archive::digest(&job_bytes) == receipt.job_ledger_sha256,
        "{label} uploaded job ledger differs from its receipt"
    );
    let job = JobLedger::parse(&job_bytes).with_context(|| format!("{label} job ledger"))?;
    ensure!(
        job.partition == scheme && job.mode == options.mode,
        "{label} job ledger names another partition"
    );
    let profiles = &receipt.profiles;
    ensure!(
        profiles.count <= PROFILE_COUNT_LIMIT && profiles.bytes <= PROFILE_TOTAL_LIMIT,
        "{label} profiles exceed their limits"
    );
    let lcov = match (&receipt.lcov, options.mode) {
        (Some(expected), Mode::Instrumented) => {
            ensure!(profiles.count > 0, "{label} recorded no raw profiles");
            let bytes = read_bounded(&directory.join(LCOV_FILE), lcov::LCOV_LIMIT)?;
            ensure!(
                bytes.len() as u64 == expected.bytes && archive::digest(&bytes) == expected.sha256,
                "{label} uploaded LCOV differs from its receipt"
            );
            let parsed = Lcov::parse(std::str::from_utf8(&bytes).context("LCOV is not UTF-8")?)
                .with_context(|| format!("{label} LCOV"))?;
            parsed.require_relative()?;
            let totals = parsed.totals();
            ensure!(
                totals.files == expected.files
                    && totals.lines_found == expected.lines_found
                    && totals.lines_hit == expected.lines_hit,
                "{label} LCOV totals differ from its receipt"
            );
            Some(parsed)
        }
        (None, Mode::Uninstrumented) => {
            ensure!(profiles.count == 0, "{label} recorded raw profiles");
            None
        }
        _ => bail!("{label} LCOV receipt does not match its mode"),
    };
    Ok(Accepted {
        index,
        attempt,
        receipt,
        plan: uploaded,
        job,
        lcov,
    })
}

/// Totals of a complete, disjoint assignment.
#[derive(Debug, Eq, PartialEq)]
struct Completeness {
    executables: usize,
    tests: usize,
    excluded: Vec<(String, String)>,
}

/// Require every executable's partitions to list identical tests and assign
/// each exactly once. Excluded executables carry one reason everywhere.
fn complete(plans: &[(u32, &PartitionPlan)]) -> Result<Completeness> {
    let (_, first) = plans.first().context("no partition plans")?;
    let mut tests = 0;
    let mut excluded = Vec::new();
    for (position, reference) in first.executables.iter().enumerate() {
        let mut owners: BTreeMap<&str, u32> = BTreeMap::new();
        for (index, plan) in plans {
            let tests = plan
                .executables
                .get(position)
                .with_context(|| format!("partition {index} plan omits {}", reference.artifact))?;
            ensure!(
                tests.artifact == reference.artifact,
                "partition {index} plans {} where partition 1 plans {}",
                tests.artifact,
                reference.artifact
            );
            ensure!(
                tests.listed == reference.listed && tests.list_sha256 == reference.list_sha256,
                "partition {index} lists different tests for {}",
                reference.artifact
            );
            ensure!(
                tests.excluded == reference.excluded,
                "partition {index} excludes {} differently",
                reference.artifact
            );
            for name in &tests.assigned {
                ensure!(
                    reference.listed.binary_search(name).is_ok(),
                    "partition {index} assigns unlisted test {name} of {}",
                    reference.artifact
                );
                if let Some(previous) = owners.insert(name, *index) {
                    bail!(
                        "test {name} of {} is assigned to partitions {previous} and {index}",
                        reference.artifact
                    );
                }
            }
        }
        match &reference.excluded {
            Some(reason) => {
                ensure!(
                    owners.is_empty(),
                    "excluded {} runs tests",
                    reference.artifact
                );
                excluded.push((reference.artifact.clone(), reason.clone()));
            }
            None => ensure!(
                owners.len() == reference.listed.len(),
                "{} of {} listed tests of {} ran in no partition",
                reference.listed.len() - owners.len(),
                reference.listed.len(),
                reference.artifact
            ),
        }
        tests += reference.listed.len();
    }
    for (index, plan) in plans {
        ensure!(
            plan.executables.len() == first.executables.len(),
            "partition {index} plans a different number of executables"
        );
    }
    Ok(Completeness {
        executables: first.executables.len(),
        tests,
        excluded,
    })
}

fn fresh_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("coverage report already exists: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("inspect the coverage report path"),
    }
}

/// Validate every partition of one OS and, only if all agree and are
/// complete, write the merged report and summary.
pub fn merge(options: &MergeOptions<'_>) -> Result<MergeSummary> {
    super::check_partitioning(options.os, options.mode, options.count)?;
    let max_attempt = canonical_attempt(options.attempt)
        .context("coverage run attempt must be a positive integer without leading zeros")?;
    let summary_path = options
        .report
        .parent()
        .context("coverage report has no parent directory")?
        .join(SUMMARY_FILE);
    fresh_path(options.report)?;
    fresh_path(&summary_path)?;
    let latest = latest_attempts(options.inputs, options.os, options.count, max_attempt)?;
    let mut accepted = Vec::new();
    for (index, (attempt, directory)) in &latest {
        accepted.push(accept(options, *index, *attempt, directory)?);
    }
    let reference = agreement(&accepted[0].receipt)?;
    for partition in &accepted[1..] {
        for ((field, expected), (_, actual)) in reference.iter().zip(agreement(&partition.receipt)?)
        {
            ensure!(
                *expected == actual,
                "coverage partition {} receipt differs from partition 1 in {field}: {actual} != {expected}",
                partition.index
            );
        }
    }
    let plans: Vec<_> = accepted
        .iter()
        .map(|partition| (partition.index, &partition.plan))
        .collect();
    let completeness = complete(&plans)?;

    let merged = match options.mode {
        Mode::Instrumented => {
            let mut lcovs = accepted.iter().map(|partition| {
                (
                    partition.index,
                    partition.lcov.as_ref().expect("instrumented LCOV"),
                )
            });
            let (_, first) = lcovs.next().expect("one partition");
            let mut merged = first.clone();
            for (index, lcov) in lcovs {
                merged.absorb(lcov, &format!("coverage partition {index}"))?;
            }
            let totals = merged.totals();
            ensure!(
                lcov::passes_gate(&totals),
                "{} line coverage {}% ({} of {} lines) is below {}%",
                options.os,
                lcov::percent(&totals),
                totals.lines_hit,
                totals.lines_found,
                lcov::LINE_GATE_PERCENT
            );
            Some((merged, totals))
        }
        Mode::Uninstrumented => None,
    };
    let summary = MergeSummary {
        schema: SCHEMA,
        os: options.os.to_owned(),
        mode: options.mode,
        source: options.source.to_owned(),
        executables: completeness.executables,
        tests: completeness.tests,
        excluded: completeness.excluded,
        partitions: accepted
            .iter()
            .map(|partition| PartitionRow {
                index: partition.index,
                attempt: partition.attempt,
                helper_cache: partition.job.cache.helper.clone(),
                seed_cache: partition.job.cache.seed.clone(),
                compile_seconds: partition.job.seconds("compile"),
                tests_seconds: partition.job.seconds("tests"),
                profiles: partition.receipt.profiles.count,
                dependency_units_rebuilt: partition.job.cache.dependency_units_rebuilt,
                slowest_executable: partition
                    .job
                    .slowest()
                    .map(|(artifact, seconds)| (artifact.to_owned(), seconds)),
            })
            .collect(),
        coverage: merged.as_ref().map(|(_, totals)| *totals),
    };
    // Written through a private name and renamed only after every check passed.
    let parent = summary_path.parent().expect("summary parent");
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let mut written = Vec::new();
    let result = (|| {
        if let Some((merged, _)) = &merged {
            let partial = parent.join(".coverage.lcov.partial");
            written.push(partial.clone());
            super::write_new(&partial, merged.render().as_bytes())?;
            fs::rename(&partial, options.report)?;
            written.push(options.report.to_path_buf());
        }
        super::write_json(&summary_path, &summary)
    })();
    if let Err(error) = result {
        for path in written {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(&summary_path);
        return Err(error);
    }
    Ok(summary)
}

/// Print the per-partition table and totals of an accepted merge.
pub fn print_summary(summary: &MergeSummary) {
    println!(
        "coverage merge {} ({}): {} executables, {} tests, disjoint and complete across {} partitions",
        summary.os,
        summary.mode.name(),
        summary.executables,
        summary.tests,
        summary.partitions.len()
    );
    println!("partition attempt helper  seed     compile_s tests_s profiles deps_rebuilt slowest");
    for row in &summary.partitions {
        let optional = |value: Option<u64>| value.map_or("-".to_owned(), |value| value.to_string());
        println!(
            "{:>9} {:>7} {:<7} {:<8} {:>9} {:>7} {:>8} {:>12} {}",
            row.index,
            row.attempt,
            row.helper_cache,
            row.seed_cache,
            optional(row.compile_seconds),
            optional(row.tests_seconds),
            row.profiles,
            row.dependency_units_rebuilt,
            row.slowest_executable
                .as_ref()
                .map_or("-".to_owned(), |(artifact, seconds)| format!(
                    "{artifact} {seconds}s"
                ))
        );
    }
    for (artifact, reason) in &summary.excluded {
        println!("excluded {artifact}: {reason}");
    }
    if let Some(totals) = &summary.coverage {
        // The union of DA records counts each instrumented source line once;
        // cargo-llvm-cov's own summary sums lines per function-instantiation
        // group and reads lower, so the two percentages are not comparable.
        println!(
            "coverage {}: {}% of unique instrumented lines (union of DA records: {} of {}) across {} files; gate {}%",
            summary.os,
            lcov::percent(totals),
            totals.lines_hit,
            totals.lines_found,
            totals.files,
            lcov::LINE_GATE_PERCENT
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{SOURCE, Workspace};
    use super::*;

    /// Every partition's evidence for one OS, placed as the download step
    /// extracts several artifacts: one directory per artifact name.
    struct Downloaded {
        workspace: Workspace,
        inputs: PathBuf,
        report: PathBuf,
        mode: Mode,
        count: u32,
        os: String,
        scope: Vec<String>,
    }

    fn name(os: &str, index: u32, attempt: u64) -> String {
        format!("ci-coverage-{os}-partition-{index}-attempt-{attempt}")
    }

    impl Downloaded {
        async fn new(mode: Mode, count: u32) -> Self {
            let workspace = Workspace::new(mode);
            let inputs = workspace.temp.path().join("inputs");
            fs::create_dir(&inputs).unwrap();
            let report = workspace.temp.path().join("report/coverage.lcov");
            let downloaded = Self {
                workspace,
                inputs,
                report,
                mode,
                count,
                os: LOCAL_OS.to_owned(),
                scope: match mode {
                    Mode::Instrumented => WORKSPACE_PACKAGES.map(str::to_owned).to_vec(),
                    Mode::Uninstrumented => vec!["kuru-memory".to_owned()],
                },
            };
            for index in 1..=count {
                downloaded.upload(index, 1).await;
            }
            downloaded
        }

        async fn upload(&self, index: u32, attempt: u64) {
            let partition = PartitionScheme::new(index, self.count).unwrap();
            self.workspace
                .evidence(
                    self.mode,
                    &partition,
                    attempt,
                    &self.inputs.join(name(&self.os, index, attempt)),
                )
                .await
                .unwrap();
        }

        fn evidence(&self, index: u32) -> PathBuf {
            self.inputs.join(name(&self.os, index, 1)).join("attempt-1")
        }

        fn options(&self) -> MergeOptions<'_> {
            MergeOptions {
                inputs: &self.inputs,
                report: &self.report,
                os: &self.os,
                source: SOURCE,
                attempt: "3",
                count: self.count,
                mode: self.mode,
                scope: &self.scope,
            }
        }

        fn edit_receipt(&self, index: u32, edit: impl FnOnce(&mut serde_json::Value)) {
            let path = self.evidence(index).join(RECEIPT_FILE);
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            edit(&mut value);
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        }

        /// Replace a partition's LCOV and rebind its receipt to it.
        fn replace_lcov(&self, index: u32, text: &str) {
            fs::write(self.evidence(index).join(LCOV_FILE), text).unwrap();
            let totals = Lcov::parse(text).unwrap().totals();
            self.edit_receipt(index, |receipt| {
                receipt["lcov"] = serde_json::json!({
                    "bytes": text.len(),
                    "sha256": archive::digest(text.as_bytes()),
                    "files": totals.files,
                    "lines_found": totals.lines_found,
                    "lines_hit": totals.lines_hit,
                });
            });
        }

        fn merge(&self) -> Result<MergeSummary> {
            merge(&self.options())
        }

        /// A failed merge leaves neither a report, a summary nor a partial file.
        fn refuse(&self, reason: &str) {
            let error = format!("{:#}", self.merge().unwrap_err());
            assert!(error.contains(reason), "{reason}: {error}");
            let parent = self.report.parent().unwrap();
            assert!(
                !parent.exists() || fs::read_dir(parent).unwrap().next().is_none(),
                "{reason}: a report file was left"
            );
        }
    }

    #[tokio::test]
    async fn agreeing_complete_partitions_merge_into_one_gated_report() {
        let downloaded = Downloaded::new(Mode::Instrumented, 3).await;
        let summary = downloaded.merge().unwrap();
        assert_eq!(summary.partitions.len(), 3);
        assert_eq!(
            summary.coverage,
            Some(Totals {
                files: 1,
                lines_found: 20,
                lines_hit: 19
            })
        );
        let runnable = downloaded.workspace.runnable().len();
        assert_eq!(summary.executables, runnable);
        assert!(summary.tests > 0);
        assert_eq!(summary.partitions[1].tests_seconds, Some(2));
        assert_eq!(summary.partitions[0].dependency_units_rebuilt, 400);
        let report = fs::read_to_string(&downloaded.report).unwrap();
        assert!(report.contains("DA:20,0\nLF:20\nLH:19\n"), "{report}");
        assert!(report.contains("FNDA:1,_RNvf\n"));
        let parent = downloaded.report.parent().unwrap();
        let written: BTreeMap<_, _> = fs::read_dir(parent)
            .unwrap()
            .map(|entry| (entry.unwrap().file_name().into_string().unwrap(), ()))
            .collect();
        assert_eq!(
            written.keys().collect::<Vec<_>>(),
            ["coverage.lcov", SUMMARY_FILE]
        );
        let stored: MergeSummary = read_versioned(&parent.join(SUMMARY_FILE), "summary").unwrap();
        assert_eq!(stored, summary);
        print_summary(&summary);
        // A second merge never replaces the first report.
        downloaded.refuse_existing();

        let memory = Downloaded::new(Mode::Uninstrumented, 2).await;
        let summary = memory.merge().unwrap();
        assert_eq!(summary.coverage, None);
        assert!(!memory.report.exists());
        assert!(memory.report.parent().unwrap().join(SUMMARY_FILE).is_file());
        print_summary(&summary);
    }

    impl Downloaded {
        fn refuse_existing(&self) {
            let error = self.merge().unwrap_err().to_string();
            assert!(error.contains("already exists"), "{error}");
        }
    }

    #[tokio::test]
    async fn receipts_that_disagree_or_misdescribe_their_evidence_fail() {
        type Edit = fn(&mut serde_json::Value);
        let cases: [(Edit, &str); 17] = [
            (|r| r["source"] = "other".into(), "was built from other"),
            (
                |r| r["tree"] = "other".into(),
                "differs from partition 1 in tree",
            ),
            (|r| r["cargo_lock_sha256"] = "x".into(), "in Cargo.lock"),
            (|r| r["rustc"] = "rustc 1.99".into(), "in rustc"),
            (|r| r["cargo"] = "cargo 1.99".into(), "in cargo:"),
            (
                |r| r["profile_env_sha256"] = "x".into(),
                "in profile environment",
            ),
            (
                |r| r["cargo_llvm_cov"] = "cargo-llvm-cov 0.9.0".into(),
                "coverage tool",
            ),
            (|r| r["mode"] = "uninstrumented".into(), "is uninstrumented"),
            (
                |r| r["instrumentation"] = "x".into(),
                "instrumentation contract",
            ),
            (
                |r| r["inventory_sha256"] = "x".into(),
                "uploaded inventory differs",
            ),
            (
                |r| r["plan_sha256"] = "x".into(),
                "uploaded plan differs from its receipt",
            ),
            (
                |r| r["tests_sha256"] = "x".into(),
                "uploaded plan differs from its receipt",
            ),
            (
                |r| r["runner_ledger_sha256"] = "x".into(),
                "runner ledger differs",
            ),
            (
                |r| r["job_ledger_sha256"] = "x".into(),
                "job ledger differs",
            ),
            (
                |r| r["lcov"]["sha256"] = "x".into(),
                "uploaded LCOV differs",
            ),
            (
                |r| r["partition"]["index"] = 1.into(),
                "carries a receipt for partition 1",
            ),
            (|r| r["run_attempt"] = "2".into(), "receipt names attempt 2"),
        ];
        let downloaded = Downloaded::new(Mode::Instrumented, 2).await;
        let receipt = fs::read(downloaded.evidence(2).join(RECEIPT_FILE)).unwrap();
        downloaded.merge().map(drop).unwrap();
        for (edit, reason) in cases {
            fs::remove_dir_all(downloaded.report.parent().unwrap()).unwrap_or(());
            downloaded.edit_receipt(2, edit);
            downloaded.refuse(reason);
            fs::write(downloaded.evidence(2).join(RECEIPT_FILE), &receipt).unwrap();
        }
        downloaded.edit_receipt(2, |r| r["target_os"] = "macos".into());
        downloaded.refuse("disagree");
        fs::write(downloaded.evidence(2).join(RECEIPT_FILE), &receipt).unwrap();
        downloaded.edit_receipt(2, |r| r["schema"] = 1.into());
        downloaded.refuse("coverage schema 1");
        fs::write(downloaded.evidence(2).join(RECEIPT_FILE), &receipt).unwrap();
        downloaded.edit_receipt(2, |r| r["lcov"] = serde_json::Value::Null);
        downloaded.refuse("LCOV receipt does not match its mode");
    }

    #[tokio::test]
    async fn tampered_or_missing_files_fail_before_any_report() {
        let downloaded = Downloaded::new(Mode::Instrumented, 2).await;
        let evidence = downloaded.evidence(1);
        let ledger = fs::read(evidence.join(RUNNER_LEDGER_FILE)).unwrap();
        fs::write(
            evidence.join(RUNNER_LEDGER_FILE),
            [&ledger[..], b"\n"].concat(),
        )
        .unwrap();
        downloaded.refuse("uploaded runner ledger differs");
        fs::write(evidence.join(RUNNER_LEDGER_FILE), &ledger).unwrap();

        let plan = fs::read_to_string(evidence.join(PLAN_FILE)).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&plan).unwrap();
        value["executables"][0]["assigned"] = serde_json::json!([]);
        fs::write(
            evidence.join(PLAN_FILE),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        downloaded.refuse("uploaded plan differs from its runner ledger");
        fs::write(evidence.join(PLAN_FILE), &plan).unwrap();

        fs::remove_file(evidence.join(JOB_LEDGER_FILE)).unwrap();
        downloaded.refuse("unexpected artifact entries");
        fs::write(evidence.join(JOB_LEDGER_FILE), b"{}").unwrap();
        downloaded.refuse("job ledger differs");

        // Another partition's evidence under this partition's name.
        let other = Downloaded::new(Mode::Instrumented, 2).await;
        let swap = other.workspace.temp.path().join("swap");
        fs::rename(other.evidence(1), &swap).unwrap();
        fs::rename(other.evidence(2), other.evidence(1)).unwrap();
        fs::rename(&swap, other.evidence(2)).unwrap();
        other.refuse("carries a receipt for partition 2");
    }

    #[tokio::test]
    async fn coverage_must_share_its_structure_and_reach_the_gate() {
        let downloaded = Downloaded::new(Mode::Instrumented, 2).await;
        let lcov = fs::read_to_string(downloaded.evidence(2).join(LCOV_FILE)).unwrap();
        downloaded.replace_lcov(2, &lcov.replace("DA:20,0\n", "DA:21,0\n"));
        downloaded.refuse("instruments different lines");
        downloaded.replace_lcov(2, &lcov.replace("DA:20,0\n", ""));
        downloaded.refuse("instruments different lines");
        let hits: String = lcov
            .lines()
            .map(|line| match line.strip_prefix("DA:") {
                Some(rest) => format!("DA:{},0\n", rest.split_once(',').unwrap().0),
                None => format!("{line}\n"),
            })
            .collect();
        let low = Downloaded::new(Mode::Instrumented, 2).await;
        low.replace_lcov(1, &hits);
        low.replace_lcov(2, &hits);
        low.refuse("line coverage 0.00% (0 of 20 lines) is below 91%");
        let absolute = Downloaded::new(Mode::Instrumented, 1).await;
        absolute.replace_lcov(1, &lcov.replace("SF:packages", "SF:/abs/packages"));
        absolute.refuse("not a normalized relative path");
    }

    #[tokio::test]
    async fn the_latest_attempt_of_every_index_is_required() {
        let downloaded = Downloaded::new(Mode::Uninstrumented, 3).await;
        // A rerun's later attempt replaces the earlier one.
        downloaded.upload(2, 3).await;
        let summary = downloaded.merge().unwrap();
        assert_eq!(
            summary
                .partitions
                .iter()
                .map(|row| (row.index, row.attempt))
                .collect::<Vec<_>>(),
            [(1, 1), (2, 3), (3, 1)]
        );
        fs::remove_dir_all(downloaded.report.parent().unwrap()).unwrap();
        // An attempt newer than the run is refused.
        downloaded.upload(3, 4).await;
        downloaded.refuse("attempt 4 is newer than run attempt 3");
        fs::remove_dir_all(downloaded.inputs.join(name(LOCAL_OS, 3, 4))).unwrap();
        // A missing index fails naming it.
        fs::remove_dir_all(downloaded.inputs.join(name(LOCAL_OS, 3, 1))).unwrap();
        downloaded.refuse("coverage partition 3 of 3 has no evidence");
    }

    #[tokio::test]
    async fn uninstrumented_partitions_must_build_the_configured_scope() {
        // Partitions that agree with each other on a scope narrower than the
        // configured packages are refused before any summary is written.
        let mut downloaded = Downloaded::new(Mode::Uninstrumented, 2).await;
        downloaded.scope = vec!["kuru-memory".to_owned(), "kuru-runtime".to_owned()];
        downloaded.refuse(
            r#"scope ["kuru-memory"] differs from the expected ["kuru-memory", "kuru-runtime"]"#,
        );
        assert!(!downloaded.report.parent().unwrap().exists());
    }

    #[test]
    fn download_layouts_are_parsed_strictly() {
        let temp = tempfile::TempDir::new().unwrap();
        let inputs = temp.path();
        let dir = |path: &str| fs::create_dir_all(inputs.join(path)).unwrap();
        let reset = || {
            for entry in fs::read_dir(inputs).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    fs::remove_dir_all(path).unwrap();
                } else {
                    fs::remove_file(path).unwrap();
                }
            }
        };
        // A single artifact extracts flat, valid only for one partition.
        dir("attempt-2");
        let latest = latest_attempts(inputs, "ubuntu-latest", 1, 2).unwrap();
        assert_eq!(latest[&1].0, 2);
        assert!(latest_attempts(inputs, "ubuntu-latest", 2, 2).is_err());
        dir(&name("ubuntu-latest", 1, 1));
        let error = latest_attempts(inputs, "ubuntu-latest", 1, 2)
            .unwrap_err()
            .to_string();
        assert!(error.contains("mix"), "{error}");
        reset();
        for (entry, inner, reason) in [
            (
                name("macos-latest", 1, 1),
                "attempt-1",
                "unexpected coverage evidence artifact",
            ),
            (
                "ci-coverage-diagnostics-ubuntu-latest-partition-1-attempt-1".to_owned(),
                "attempt-1",
                "unexpected coverage evidence artifact",
            ),
            (
                "-coverage-ubuntu-latest-partition-1-attempt-1".to_owned(),
                "attempt-1",
                "unexpected",
            ),
            (name("ubuntu-latest", 0, 1), "attempt-1", "unexpected"),
            (name("ubuntu-latest", 3, 1), "attempt-1", "outside 1..=2"),
            (name("ubuntu-latest", 1, 1), "attempt-2", "holds attempt-2"),
            (
                name("ubuntu-latest", 1, 1),
                "evidence",
                "not a canonical attempt",
            ),
            (
                "ci-coverage-ubuntu-latest-partition-1-attempt-01".to_owned(),
                "attempt-1",
                "unexpected",
            ),
        ] {
            dir(&format!("{entry}/{inner}"));
            let error = latest_attempts(inputs, "ubuntu-latest", 2, 3)
                .unwrap_err()
                .to_string();
            assert!(error.contains(reason), "{entry}: {error}");
            reset();
        }
        dir(&format!("{}/attempt-1", name("ubuntu-latest", 1, 1)));
        dir(&format!("{}/attempt-2", name("ubuntu-latest", 1, 1)));
        assert!(
            latest_attempts(inputs, "ubuntu-latest", 1, 3)
                .unwrap_err()
                .to_string()
                .contains("must hold one attempt directory")
        );
        reset();
        dir(&format!("{}/attempt-1", name("ubuntu-latest", 1, 1)));
        dir("dl-coverage-ubuntu-latest-partition-2-attempt-1/attempt-1");
        assert!(
            latest_attempts(inputs, "ubuntu-latest", 2, 3)
                .unwrap_err()
                .to_string()
                .contains("different prefixes")
        );
        reset();
        fs::write(inputs.join("stray"), b"").unwrap();
        assert!(
            latest_attempts(inputs, "ubuntu-latest", 1, 3)
                .unwrap_err()
                .to_string()
                .contains("is not a directory")
        );
        assert!(latest_attempts(&inputs.join("missing"), "ubuntu-latest", 1, 3).is_err());
        // Dotted labels, as the arm64 runner's, parse unambiguously.
        assert_eq!(
            artifact_name(&name("ubuntu-24.04-arm", 3, 2), "ubuntu-24.04-arm").unwrap(),
            ("ci", 3, 2)
        );
        assert!(artifact_name(&name("ubuntu-24.04-arm", 3, 2), "arm").is_err());
    }

    #[tokio::test]
    async fn partitions_must_run_on_their_labels_host() {
        let mut downloaded = Downloaded::new(Mode::Uninstrumented, 3).await;
        downloaded.os = "ubuntu-24.04-arm".to_owned();
        for index in 1..=3 {
            fs::rename(
                downloaded.inputs.join(name(LOCAL_OS, index, 1)),
                downloaded.inputs.join(name(&downloaded.os, index, 1)),
            )
            .unwrap();
        }
        downloaded.refuse("but ubuntu-24.04-arm partitions run on aarch64-unknown-linux-gnu");
        downloaded.count = 2;
        downloaded.refuse("partitions number 3, not 2");
        assert_eq!(target_os("aarch64-pc-windows-msvc"), Some("windows"));
        assert_eq!(target_os("wasm32-unknown-unknown"), None);
    }

    fn tests(artifact: &str, listed: &[&str], assigned: &[&str]) -> plan::ExecutableTests {
        let listed: Vec<String> = listed.iter().map(|name| (*name).to_owned()).collect();
        let assigned: Vec<String> = assigned.iter().map(|name| (*name).to_owned()).collect();
        plan::ExecutableTests {
            artifact: artifact.to_owned(),
            executable: format!("debug/deps/{artifact}"),
            list_sha256: super::super::partition::list_sha256(&listed),
            assigned_sha256: super::super::partition::list_sha256(&assigned),
            listed,
            assigned,
            excluded: None,
        }
    }

    fn plan(index: u32, executables: Vec<plan::ExecutableTests>) -> PartitionPlan {
        PartitionPlan {
            schema: SCHEMA,
            inventory_sha256: "inventory".to_owned(),
            partition: PartitionScheme::new(index, 2).unwrap(),
            executables,
        }
    }

    #[test]
    fn plans_must_be_disjoint_and_complete() {
        let one = plan(
            1,
            vec![tests("a", &["x", "y"], &["x"]), tests("b", &[], &[])],
        );
        let two = plan(
            2,
            vec![tests("a", &["x", "y"], &["y"]), tests("b", &[], &[])],
        );
        assert_eq!(
            complete(&[(1, &one), (2, &two)]).unwrap(),
            Completeness {
                executables: 2,
                tests: 2,
                excluded: Vec::new()
            }
        );
        for (other, reason) in [
            (
                plan(
                    2,
                    vec![tests("a", &["x", "y"], &["x"]), tests("b", &[], &[])],
                ),
                "assigned to partitions 1 and 2",
            ),
            (
                plan(2, vec![tests("a", &["x", "y"], &[]), tests("b", &[], &[])]),
                "1 of 2 listed tests of a ran in no partition",
            ),
            (
                plan(
                    2,
                    vec![tests("a", &["x", "z"], &["z"]), tests("b", &[], &[])],
                ),
                "lists different tests",
            ),
            (
                plan(2, vec![tests("a", &["x", "y"], &["y"])]),
                "plan omits b",
            ),
            (
                plan(
                    2,
                    vec![tests("c", &["x", "y"], &["y"]), tests("b", &[], &[])],
                ),
                "plans c where partition 1 plans a",
            ),
            (
                plan(
                    2,
                    vec![tests("a", &["x", "y"], &["y", "w"]), tests("b", &[], &[])],
                ),
                "assigns unlisted test w",
            ),
            (
                plan(
                    2,
                    vec![
                        tests("a", &["x", "y"], &["y"]),
                        tests("b", &[], &[]),
                        tests("c", &[], &[]),
                    ],
                ),
                "different number of executables",
            ),
        ] {
            let error = complete(&[(1, &one), (2, &other)]).unwrap_err().to_string();
            assert!(error.contains(reason), "{reason}: {error}");
        }
        let mut excluded_one = tests("a", &["x", "y"], &[]);
        excluded_one.excluded = Some("reason".to_owned());
        let mut excluded_two = excluded_one.clone();
        let one = plan(1, vec![excluded_one.clone()]);
        let two = plan(2, vec![excluded_two.clone()]);
        assert_eq!(
            complete(&[(1, &one), (2, &two)]).unwrap().excluded,
            [("a".to_owned(), "reason".to_owned())]
        );
        excluded_two.excluded = Some("other".to_owned());
        let two = plan(2, vec![excluded_two]);
        assert!(
            complete(&[(1, &one), (2, &two)])
                .unwrap_err()
                .to_string()
                .contains("excludes a differently")
        );
        let mut running = excluded_one;
        running.assigned = vec!["x".to_owned()];
        let two = plan(2, vec![running]);
        assert!(
            complete(&[(1, &one), (2, &two)])
                .unwrap_err()
                .to_string()
                .contains("excluded a runs tests")
        );
        assert!(complete(&[]).is_err());
    }
}
