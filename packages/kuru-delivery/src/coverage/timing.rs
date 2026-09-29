//! Duration-aware partition placement from a checked-in timing table.
//!
//! [`TABLE_PATH`] holds, for each artifact key and test name, the milliseconds
//! that recorded runs attributed to the test on each hosted OS label. Every
//! partition of an OS, and the merge that proves them again, places the
//! table's tests for that label with one greedy longest-processing-time pass:
//! heaviest first, ties by artifact key and then test name, each to the least
//! loaded partition, ties to the lowest index. A listed test without a row
//! keeps the SHA-256 name hash of [`partition::assign`]. The placement is a
//! pure function of the table, the OS label and the partition count, so every
//! test has exactly one owner. The merge still proves the plans disjoint and
//! complete, exactly as before.
//!
//! Attribution: libtest prints one completion line per test. Within one exact
//! selection, a test is attributed the time from the previous completion (or
//! the selection's start) to its own. The attributed times of a selection
//! therefore sum to its wall time up to its last completion.

use super::{
    ATTEMPT_DIRECTORY, OS_TARGETS, PARTITIONS, RECEIPT_FILE, RUNNER_LEDGER_FILE,
    RUNNER_LEDGER_LIMIT, Receipt, archive, canonical_attempt,
    partition::{self, MAX_PARTITIONS},
    plan::{self, CompletedTest, PartitionPlan},
    read_bounded, read_versioned,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// The checked-in table, relative to the repository root.
pub const TABLE_PATH: &str = "packages/kuru-delivery/src/coverage/timings.tsv";
const EMBEDDED: &str = include_str!("timings.tsv");
/// The largest weight a row may carry: one day.
pub const MAX_WEIGHT_MS: u64 = 24 * 60 * 60 * 1000;
/// The weight predictions give an unknown test when a label has no rows.
pub const FALLBACK_WEIGHT_MS: u64 = 1000;
/// The merge warns when more than this percentage of an OS's listed tests
/// have no row for its label.
pub const STALE_PERCENT: usize = 5;
/// Unknown test names the job summary lists before it abbreviates.
const SUMMARY_NAME_LIMIT: usize = 50;
const PREAMBLE: &str = "\
# Attributed test milliseconds per hosted OS label, read by duration-aware
# partition placement. Regenerate it from a completed run's partition evidence
# with `mise run //packages/kuru-delivery:coverage:timings -- --inputs <dir>`
# (docs/development.md). \"-\" marks a test the label did not run or time.
";

/// One test's attributed milliseconds per table label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub artifact: String,
    pub test: String,
    pub weights: Vec<Option<u64>>,
}

/// A parsed timing table: hosted OS labels and rows sorted by artifact key and
/// test name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimingTable {
    labels: Vec<String>,
    rows: Vec<Row>,
}

/// The hosted OS label of a Rust host target, if it has one.
pub fn label_for_host(host: &str) -> Option<&'static str> {
    OS_TARGETS
        .iter()
        .find(|(_, target)| *target == host)
        .map(|(label, _)| *label)
}

fn label_position(label: &str) -> Option<usize> {
    OS_TARGETS.iter().position(|(hosted, _)| *hosted == label)
}

fn check_artifact(artifact: &str) -> Result<()> {
    let parts: Vec<_> = artifact.split('/').collect();
    ensure!(
        parts.len() == 3
            && parts.iter().all(|part| !part.is_empty())
            && !artifact
                .chars()
                .any(|character| character.is_whitespace() || character.is_control()),
        "invalid artifact key {artifact:?}"
    );
    Ok(())
}

fn check_test(test: &str) -> Result<()> {
    ensure!(
        !test.is_empty() && test.trim() == test && !test.chars().any(char::is_control),
        "invalid test name {test:?}"
    );
    Ok(())
}

fn parse_weight(text: &str) -> Result<Option<u64>> {
    if text == "-" {
        return Ok(None);
    }
    ensure!(
        !text.is_empty()
            && text.bytes().all(|byte| byte.is_ascii_digit())
            && (text == "0" || !text.starts_with('0')),
        "invalid weight {text:?}"
    );
    let weight: u64 = text.parse().with_context(|| format!("weight {text:?}"))?;
    ensure!(
        weight <= MAX_WEIGHT_MS,
        "weight {weight} exceeds {MAX_WEIGHT_MS} ms"
    );
    Ok(Some(weight))
}

impl TimingTable {
    /// Parse a table strictly: `#` comments only before the header, a header
    /// of hosted labels in [`OS_TARGETS`] order, rows with one cell per label
    /// and at least one weight, sorted and unique by artifact key and test.
    /// A trailing carriage return per line is ignored, so the canonical form
    /// and digest do not depend on line endings.
    pub fn parse(text: &str) -> Result<Self> {
        let body = text
            .strip_suffix('\n')
            .context("timing table must end with a newline")?;
        let mut lines = body
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .enumerate()
            .skip_while(|(_, line)| line.starts_with('#'));
        let (_, header) = lines.next().context("timing table has no header")?;
        let fields: Vec<_> = header.split('\t').collect();
        ensure!(
            fields.len() > 2 && fields[..2] == ["artifact", "test"],
            "timing table header must be `artifact`, `test` and OS labels"
        );
        let mut labels = Vec::new();
        let mut previous = None;
        for label in &fields[2..] {
            let position = label_position(label)
                .with_context(|| format!("timing table names unknown OS label {label:?}"))?;
            ensure!(
                previous.is_none_or(|previous| previous < position),
                "timing table labels must be unique and in the hosted table's order"
            );
            previous = Some(position);
            labels.push((*label).to_owned());
        }
        let mut rows: Vec<Row> = Vec::new();
        for (index, line) in lines {
            let number = index + 1;
            let cells: Vec<_> = line.split('\t').collect();
            ensure!(
                cells.len() == labels.len() + 2,
                "timing table line {number} has {} cells, expected {}",
                cells.len(),
                labels.len() + 2
            );
            check_artifact(cells[0]).with_context(|| format!("timing table line {number}"))?;
            check_test(cells[1]).with_context(|| format!("timing table line {number}"))?;
            let weights = cells[2..]
                .iter()
                .map(|cell| parse_weight(cell))
                .collect::<Result<Vec<_>>>()
                .with_context(|| format!("timing table line {number}"))?;
            ensure!(
                weights.iter().any(Option::is_some),
                "timing table line {number} has no weight"
            );
            let row = Row {
                artifact: cells[0].to_owned(),
                test: cells[1].to_owned(),
                weights,
            };
            if let Some(last) = rows.last() {
                ensure!(
                    (&last.artifact, &last.test) < (&row.artifact, &row.test),
                    "timing table line {number} is out of order or repeated"
                );
            }
            rows.push(row);
        }
        Ok(Self { labels, rows })
    }

    /// Build a table from rows keyed by artifact and test, dropping rows
    /// without a weight.
    fn from_rows(labels: Vec<String>, rows: BTreeMap<(String, String), Vec<Option<u64>>>) -> Self {
        Self {
            labels,
            rows: rows
                .into_iter()
                .filter(|(_, weights)| weights.iter().any(Option::is_some))
                .map(|((artifact, test), weights)| Row {
                    artifact,
                    test,
                    weights,
                })
                .collect(),
        }
    }

    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    fn canonical(&self) -> String {
        let mut text = String::from("artifact\ttest");
        for label in &self.labels {
            text.push('\t');
            text.push_str(label);
        }
        text.push('\n');
        for row in &self.rows {
            text.push_str(&row.artifact);
            text.push('\t');
            text.push_str(&row.test);
            for weight in &row.weights {
                text.push('\t');
                match weight {
                    Some(weight) => text.push_str(&weight.to_string()),
                    None => text.push('-'),
                }
            }
            text.push('\n');
        }
        text
    }

    /// The file form: the fixed preamble and the canonical header and rows.
    pub fn render(&self) -> String {
        format!("{PREAMBLE}{}", self.canonical())
    }

    /// Digest of the canonical header and rows, without comments.
    pub fn digest(&self) -> String {
        archive::digest(self.canonical().as_bytes())
    }

    fn column(&self, label: &str) -> Option<usize> {
        self.labels.iter().position(|column| column == label)
    }

    /// The rows timed on a label: artifact key, test name and weight.
    pub fn weights(&self, label: &str) -> Vec<(&str, &str, u64)> {
        let Some(column) = self.column(label) else {
            return Vec::new();
        };
        self.rows
            .iter()
            .filter_map(|row| {
                row.weights[column].map(|weight| (row.artifact.as_str(), row.test.as_str(), weight))
            })
            .collect()
    }

    /// The weight a label's row gives a test, if it has one.
    pub fn weight(&self, label: &str, artifact: &str, test: &str) -> Option<u64> {
        let column = self.column(label)?;
        let index = self
            .rows
            .binary_search_by(|row| {
                (row.artifact.as_str(), row.test.as_str()).cmp(&(artifact, test))
            })
            .ok()?;
        self.rows[index].weights[column]
    }

    /// The weight predictions give a test without a row: the label's mean.
    pub fn default_weight(&self, label: &str) -> u64 {
        let weights = self.weights(label);
        if weights.is_empty() {
            return FALLBACK_WEIGHT_MS;
        }
        let total: u64 = weights.iter().map(|(_, _, weight)| weight).sum();
        let count = weights.len() as u64;
        (total + count / 2) / count
    }
}

/// The checked-in table and its digest, parsed once per process.
fn embedded_entry() -> Result<&'static (TimingTable, String)> {
    static TABLE: OnceLock<std::result::Result<(TimingTable, String), String>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            TimingTable::parse(EMBEDDED)
                .map(|table| {
                    let digest = table.digest();
                    (table, digest)
                })
                .map_err(|error| format!("{error:#}"))
        })
        .as_ref()
        .map_err(|error| anyhow!("the checked-in timing table {TABLE_PATH} is invalid: {error}"))
}

/// The checked-in table.
pub fn embedded() -> Result<&'static TimingTable> {
    Ok(&embedded_entry()?.0)
}

/// The canonical digest of the checked-in table.
pub fn embedded_sha256() -> Result<&'static str> {
    Ok(&embedded_entry()?.1)
}

/// One partition count's placement of a label's timed tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Placement {
    count: u32,
    placed: BTreeMap<String, BTreeMap<String, u32>>,
    loads: Vec<u64>,
}

impl Placement {
    /// Greedy longest-processing-time placement. The input order does not
    /// matter: rows are ordered by descending weight, then artifact key, then
    /// test name, and each goes to the least-loaded partition, the lowest
    /// index on a tie. A repeated row fails.
    pub fn new<'a>(
        rows: impl IntoIterator<Item = (&'a str, &'a str, u64)>,
        count: u32,
    ) -> Result<Self> {
        ensure!(
            (1..=MAX_PARTITIONS).contains(&count),
            "partition count {count} is outside 1..={MAX_PARTITIONS}"
        );
        let mut rows: Vec<_> = rows.into_iter().collect();
        rows.sort_by(|left, right| {
            right
                .2
                .cmp(&left.2)
                .then_with(|| left.0.cmp(right.0))
                .then_with(|| left.1.cmp(right.1))
        });
        let mut loads = vec![0_u64; count as usize];
        let mut placed: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
        for (artifact, test, weight) in rows {
            let (slot, _) = loads
                .iter()
                .enumerate()
                .min_by_key(|(slot, load)| (**load, *slot))
                .expect("at least one partition");
            loads[slot] = loads[slot].saturating_add(weight);
            // The slot is below `count`, which is a `u32`.
            let index = slot as u32 + 1;
            ensure!(
                placed
                    .entry(artifact.to_owned())
                    .or_default()
                    .insert(test.to_owned(), index)
                    .is_none(),
                "timing rows repeat {artifact} {test}"
            );
        }
        Ok(Self {
            count,
            placed,
            loads,
        })
    }

    /// The placement of a table's rows for a host target's OS label. A host
    /// without a hosted label, or a label without a column, places nothing,
    /// so every test keeps its hash partition.
    pub fn for_host(table: &TimingTable, host: &str, count: u32) -> Result<Self> {
        let rows = label_for_host(host)
            .map(|label| table.weights(label))
            .unwrap_or_default();
        Self::new(rows, count)
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    /// The partition the table places a test in, if it has a row.
    pub fn placed(&self, artifact: &str, test: &str) -> Option<u32> {
        self.placed.get(artifact)?.get(test).copied()
    }

    /// The one-based partition of a test: its placement, or its hash.
    pub fn owner(&self, artifact: &str, test: &str) -> u32 {
        self.placed(artifact, test)
            .unwrap_or_else(|| partition::assign(artifact, test, self.count))
    }

    /// Predicted milliseconds per partition from the placed rows alone.
    pub fn loads(&self) -> &[u64] {
        &self.loads
    }
}

/// Attribute a selection's completions: each test gets the time since the
/// previous completion, in completion order (ties by name).
pub fn attribute(completed: &[CompletedTest]) -> Vec<(&str, u64)> {
    let mut ordered: Vec<_> = completed.iter().collect();
    ordered.sort_by(|left, right| {
        left.millis
            .cmp(&right.millis)
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut previous = 0;
    ordered
        .into_iter()
        .map(|test| {
            let weight = test.millis.saturating_sub(previous);
            previous = previous.max(test.millis);
            (test.name.as_str(), weight)
        })
        .collect()
}

/// Attributed milliseconds of one run by OS label, artifact key and test.
pub type Measured = BTreeMap<String, BTreeMap<(String, String), u64>>;

/// Every `attempt-<n>` evidence directory under a download directory, in
/// either download layout.
fn evidence_directories(inputs: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(inputs)
        .with_context(|| format!("read {}", inputs.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    entries.sort();
    for entry in entries {
        if !fs::symlink_metadata(&entry)?.file_type().is_dir() {
            continue;
        }
        let candidates = if is_attempt(&entry) {
            vec![entry]
        } else {
            let mut inner: Vec<_> = fs::read_dir(&entry)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<_>>()?;
            inner.sort();
            inner
        };
        for candidate in candidates {
            if is_attempt(&candidate)
                && fs::symlink_metadata(&candidate)?.file_type().is_dir()
                && fs::symlink_metadata(candidate.join(RECEIPT_FILE))
                    .is_ok_and(|metadata| metadata.file_type().is_file())
            {
                found.push(candidate);
            }
        }
    }
    Ok(found)
}

fn is_attempt(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(ATTEMPT_DIRECTORY))
        .and_then(canonical_attempt)
        .is_some()
}

/// Read the partition evidence of one completed run and attribute every
/// recorded completion. Only each partition's highest attempt counts. The
/// evidence must be one run's: every receipt names the same source and tree,
/// and each measured label has one partition count and every index up to it,
/// because [`refresh`] replaces a measured label's whole column. A missing
/// partition, mixed evidence, a runner ledger that differs from its receipt, a
/// host without a hosted label, or a test measured twice for one label fails.
pub fn measure(inputs: &Path) -> Result<Measured> {
    let mut latest: BTreeMap<(&'static str, u32), (u64, PathBuf, Receipt)> = BTreeMap::new();
    for directory in evidence_directories(inputs)? {
        let receipt: Receipt = read_versioned(&directory.join(RECEIPT_FILE), "coverage receipt")?;
        let label = label_for_host(&receipt.target).with_context(|| {
            format!(
                "{} ran on {}, which has no hosted OS label",
                directory.display(),
                receipt.target
            )
        })?;
        let attempt = canonical_attempt(&receipt.run_attempt)
            .with_context(|| format!("{} names no run attempt", directory.display()))?;
        let key = (label, receipt.partition.index);
        match latest.get(&key) {
            Some((existing, _, _)) if *existing > attempt => {}
            Some((existing, _, _)) if *existing == attempt => bail!(
                "{label} partition {} attempt {attempt} appears twice",
                receipt.partition.index
            ),
            _ => {
                latest.insert(key, (attempt, directory, receipt));
            }
        }
    }
    ensure!(
        !latest.is_empty(),
        "{} holds no partition evidence",
        inputs.display()
    );
    check_one_run(&latest)?;
    let mut measured = Measured::new();
    for ((label, index), (_, directory, receipt)) in latest {
        let path = directory.join(RUNNER_LEDGER_FILE);
        ensure!(
            archive::digest(&read_bounded(&path, RUNNER_LEDGER_LIMIT)?)
                == receipt.runner_ledger_sha256,
            "{} differs from its receipt",
            path.display()
        );
        let tests = measured.entry(label.to_owned()).or_default();
        for record in plan::read_ledger(&path)? {
            for invocation in record
                .invocations
                .iter()
                .filter(|invocation| invocation.kind == plan::EXACT)
            {
                for (test, weight) in attribute(&invocation.completed) {
                    ensure!(
                        tests
                            .insert((record.artifact.clone(), test.to_owned()), weight)
                            .is_none(),
                        "{label} partition {index} measures {} {test} twice",
                        record.artifact
                    );
                }
            }
        }
    }
    Ok(measured)
}

/// Require the latest evidence of every partition to be one complete run:
/// one source and tree across every receipt, and for each label one partition
/// count with every index from 1 to it.
fn check_one_run(latest: &BTreeMap<(&'static str, u32), (u64, PathBuf, Receipt)>) -> Result<()> {
    let mut receipts = latest
        .values()
        .map(|(_, directory, receipt)| (directory, receipt));
    let Some((first_directory, first)) = receipts.next() else {
        return Ok(());
    };
    for (directory, receipt) in receipts {
        ensure!(
            receipt.source == first.source && receipt.tree == first.tree,
            "{} measures source {} tree {}, but {} measures source {} tree {}; give one run's evidence per --inputs directory",
            directory.display(),
            receipt.source,
            receipt.tree,
            first_directory.display(),
            first.source,
            first.tree
        );
    }
    let mut counts: BTreeMap<&str, (u32, BTreeSet<u32>)> = BTreeMap::new();
    for ((label, index), (_, directory, receipt)) in latest {
        let (count, indexes) = counts
            .entry(*label)
            .or_insert_with(|| (receipt.partition.count, BTreeSet::new()));
        ensure!(
            receipt.partition.count == *count,
            "{label} evidence names partition counts {count} and {} ({})",
            receipt.partition.count,
            directory.display()
        );
        ensure!(
            (1..=*count).contains(index),
            "{label} evidence names partition {index} of {count} ({})",
            directory.display()
        );
        indexes.insert(*index);
    }
    for (label, (count, indexes)) in counts {
        let missing: Vec<u32> = (1..=count)
            .filter(|index| !indexes.contains(index))
            .collect();
        ensure!(
            missing.is_empty(),
            "{label} evidence lacks partitions {missing:?} of {count}; refreshing from it would drop their tests' rows, so download a run whose {label} partitions all uploaded evidence"
        );
    }
    Ok(())
}

/// Refresh a table from measured runs, newest first. Each label the newest
/// run measured gets a new column: exactly the tests that run measured in all
/// of its partitions, which [`measure`] requires, each the rounded mean of
/// every given run that measured it. Every other label
/// keeps the base column. Rows left without any weight are dropped.
pub fn refresh(base: &TimingTable, runs: &[Measured]) -> Result<TimingTable> {
    let (newest, _) = runs.split_first().context("no measured run was given")?;
    let labels: Vec<String> = OS_TARGETS
        .iter()
        .map(|(label, _)| (*label).to_owned())
        .filter(|label| base.labels.contains(label) || newest.contains_key(label))
        .collect();
    let mut rows: BTreeMap<(String, String), Vec<Option<u64>>> = BTreeMap::new();
    for (column, label) in labels.iter().enumerate() {
        let mut set = |key: (String, String), weight: u64| {
            rows.entry(key).or_insert_with(|| vec![None; labels.len()])[column] = Some(weight);
        };
        match newest.get(label) {
            Some(measured) => {
                for key in measured.keys() {
                    let samples: Vec<u64> = runs
                        .iter()
                        .filter_map(|run| run.get(label).and_then(|tests| tests.get(key)))
                        .copied()
                        .collect();
                    let count = samples.len() as u64;
                    let mean = (samples.iter().sum::<u64>() + count / 2) / count;
                    ensure!(
                        mean <= MAX_WEIGHT_MS,
                        "{label} {} {} measures {mean} ms",
                        key.0,
                        key.1
                    );
                    set(key.clone(), mean);
                }
            }
            None => {
                for (artifact, test, weight) in base.weights(label) {
                    set((artifact.to_owned(), test.to_owned()), weight);
                }
            }
        }
    }
    let table = TimingTable::from_rows(labels, rows);
    // The refreshed table must satisfy its own parser.
    TimingTable::parse(&table.render()).context("the refreshed timing table is invalid")?;
    Ok(table)
}

/// Predicted partition totals of one hosted label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Balance {
    pub label: String,
    pub count: u32,
    pub rows: usize,
    pub loads: Vec<u64>,
}

/// The predicted partition totals of every hosted label with a column, at
/// its hosted partition count.
pub fn balance(table: &TimingTable) -> Result<Vec<Balance>> {
    let mut balances = Vec::new();
    for (label, _, count) in PARTITIONS {
        if table.column(label).is_none() {
            continue;
        }
        let rows = table.weights(label);
        let placement = Placement::new(rows.iter().copied(), count)?;
        balances.push(Balance {
            label: label.to_owned(),
            count,
            rows: rows.len(),
            loads: placement.loads().to_vec(),
        });
    }
    Ok(balances)
}

/// One line per hosted label: rows, predicted partition seconds and the
/// largest, which bounds that label's test phase.
pub fn describe(table: &TimingTable) -> Result<String> {
    let mut text = format!("timing table {}\n", table.digest());
    for balance in balance(table)? {
        let largest = balance.loads.iter().copied().max().unwrap_or_default();
        text.push_str(&format!(
            "{} ({} partitions, {} rows): predicted test seconds {}; largest {}\n",
            balance.label,
            balance.count,
            balance.rows,
            balance
                .loads
                .iter()
                .map(|millis| seconds(*millis))
                .collect::<Vec<_>>()
                .join(" "),
            seconds(largest)
        ));
    }
    Ok(text)
}

/// Rewrite the table file from the downloaded partition evidence of one or
/// more completed runs, newest first (see [`refresh`]), and describe it.
pub fn regenerate(table: &Path, inputs: &[PathBuf]) -> Result<String> {
    let text = fs::read_to_string(table).with_context(|| format!("read {}", table.display()))?;
    let base = TimingTable::parse(&text).with_context(|| format!("parse {}", table.display()))?;
    let runs = inputs
        .iter()
        .map(|inputs| measure(inputs))
        .collect::<Result<Vec<_>>>()?;
    let refreshed = refresh(&base, &runs)?;
    let partial = table.with_extension("tsv.partial");
    fs::write(&partial, refreshed.render())
        .with_context(|| format!("write {}", partial.display()))?;
    fs::rename(&partial, table).with_context(|| format!("replace {}", table.display()))?;
    describe(&refreshed)
}

/// Seconds, to one decimal, of a millisecond total.
pub fn seconds(millis: u64) -> String {
    format!("{}.{}", millis / 1000, (millis % 1000) / 100)
}

/// How well the table describes one OS's merged plans.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimingReport {
    /// The OS label whose column placed the tests, if the host has one.
    pub label: Option<String>,
    pub table_sha256: String,
    /// The weight counted for a test without a row.
    pub default_weight_ms: u64,
    /// Listed tests of executables that are not excluded.
    pub listed: usize,
    /// Those without a row, as `artifact test`, sorted.
    pub unknown: Vec<String>,
    /// Rows of the label that name no listed test.
    pub stale_rows: usize,
    /// Predicted milliseconds per partition, in index order.
    pub predicted_ms: Vec<u64>,
    /// Set when unknown tests exceed [`STALE_PERCENT`] of the listed tests.
    pub warning: Option<String>,
}

/// Compare an OS's merged plans with the table that placed them.
pub fn report(
    table: &TimingTable,
    digest: &str,
    host: &str,
    count: u32,
    plans: &[(u32, &PartitionPlan)],
) -> TimingReport {
    let label = label_for_host(host);
    let default_weight_ms = label.map_or(FALLBACK_WEIGHT_MS, |label| table.default_weight(label));
    let weight =
        |artifact: &str, test: &str| label.and_then(|label| table.weight(label, artifact, test));
    let mut predicted_ms = vec![0_u64; count as usize];
    let mut listed = 0;
    let mut unknown = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some((_, first)) = plans.first() {
        for tests in first
            .executables
            .iter()
            .filter(|tests| tests.excluded.is_none())
        {
            for test in &tests.listed {
                listed += 1;
                seen.insert((tests.artifact.as_str(), test.as_str()));
                if weight(&tests.artifact, test).is_none() {
                    unknown.push(format!("{} {test}", tests.artifact));
                }
            }
        }
    }
    for (index, plan) in plans {
        let Some(slot) = predicted_ms.get_mut((*index as usize).wrapping_sub(1)) else {
            continue;
        };
        for tests in &plan.executables {
            for test in &tests.assigned {
                *slot =
                    slot.saturating_add(weight(&tests.artifact, test).unwrap_or(default_weight_ms));
            }
        }
    }
    let stale_rows = label.map_or(0, |label| {
        table
            .weights(label)
            .iter()
            .filter(|(artifact, test, _)| !seen.contains(&(*artifact, *test)))
            .count()
    });
    let warning = (unknown.len() * 100 > listed * STALE_PERCENT).then(|| {
        format!(
            "{} of {listed} listed tests have no timing row for {}; they keep their hash partition. Refresh {TABLE_PATH} with `mise run //packages/kuru-delivery:coverage:timings`.",
            unknown.len(),
            label.unwrap_or("this host")
        )
    });
    TimingReport {
        label: label.map(str::to_owned),
        table_sha256: digest.to_owned(),
        default_weight_ms,
        listed,
        unknown,
        stale_rows,
        predicted_ms,
        warning,
    }
}

impl TimingReport {
    /// A Markdown job-summary section; `measured` holds each partition's
    /// measured test seconds, in index order, where known.
    pub fn markdown(&self, os: &str, measured: &[Option<u64>]) -> String {
        let mut text = format!(
            "### Partition timing ({os})\n\nTable `{}` ({}), column `{}`. A test without a row counts {} s.\n\n",
            &self.table_sha256[..self.table_sha256.len().min(12)],
            TABLE_PATH,
            self.label.as_deref().unwrap_or("none"),
            seconds(self.default_weight_ms)
        );
        if let Some(warning) = &self.warning {
            text.push_str(&format!("> [!WARNING]\n> {warning}\n\n"));
        }
        text.push_str("| Partition | Predicted test s | Measured test s |\n| --- | --- | --- |\n");
        for (slot, predicted) in self.predicted_ms.iter().enumerate() {
            let measured = measured
                .get(slot)
                .copied()
                .flatten()
                .map_or("-".to_owned(), |seconds| seconds.to_string());
            text.push_str(&format!(
                "| {} | {} | {measured} |\n",
                slot + 1,
                seconds(*predicted)
            ));
        }
        text.push_str(&format!(
            "\n{} listed tests; {} without a row; {} rows name no listed test.\n",
            self.listed,
            self.unknown.len(),
            self.stale_rows
        ));
        if !self.unknown.is_empty() {
            text.push_str("\nWithout a row:\n\n");
            for name in self.unknown.iter().take(SUMMARY_NAME_LIMIT) {
                text.push_str(&format!("- `{name}`\n"));
            }
            if self.unknown.len() > SUMMARY_NAME_LIMIT {
                text.push_str(&format!(
                    "- and {} more\n",
                    self.unknown.len() - SUMMARY_NAME_LIMIT
                ));
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "artifact\ttest\tubuntu-latest\twindows-latest\n";

    fn table(rows: &str) -> TimingTable {
        TimingTable::parse(&format!("# note\n{HEADER}{rows}")).unwrap()
    }

    #[test]
    fn the_checked_in_table_parses_and_covers_every_hosted_label() {
        let table = embedded().unwrap();
        assert_eq!(
            table.labels(),
            OS_TARGETS.map(|(label, _)| label.to_owned()),
            "{TABLE_PATH}"
        );
        assert!(table.rows().len() > 100);
        assert_eq!(embedded_sha256().unwrap(), table.digest());
        // The file is in its canonical form, so a refresh diffs only data.
        assert_eq!(table.render(), EMBEDDED);
        for balance in balance(table).unwrap() {
            assert!(balance.rows > 0, "{}", balance.label);
            assert_eq!(balance.loads.len(), balance.count as usize);
        }
    }

    #[test]
    fn tables_parse_strictly_and_digest_canonically() {
        let parsed = table("a/lib/a\tt::x\t5\t-\na/lib/a\tt::y\t0\t7\n");
        assert_eq!(
            parsed.weights("ubuntu-latest"),
            [("a/lib/a", "t::x", 5), ("a/lib/a", "t::y", 0)]
        );
        assert_eq!(parsed.weights("windows-latest"), [("a/lib/a", "t::y", 7)]);
        assert!(parsed.weights("macos-latest").is_empty());
        assert_eq!(parsed.weight("windows-latest", "a/lib/a", "t::y"), Some(7));
        assert_eq!(parsed.weight("windows-latest", "a/lib/a", "t::x"), None);
        assert_eq!(parsed.weight("macos-latest", "a/lib/a", "t::y"), None);
        assert_eq!(parsed.default_weight("ubuntu-latest"), 3);
        assert_eq!(parsed.default_weight("macos-latest"), FALLBACK_WEIGHT_MS);
        // Comments and line endings do not change the digest.
        let crlf = format!("{HEADER}a/lib/a\tt::x\t5\t-\r\na/lib/a\tt::y\t0\t7\r\n");
        assert_eq!(TimingTable::parse(&crlf).unwrap().digest(), parsed.digest());
        assert_eq!(TimingTable::parse(&parsed.render()).unwrap(), parsed);
        assert_ne!(table("a/lib/a\tt::x\t6\t-\n").digest(), parsed.digest());
        for (text, reason) in [
            ("", "newline"),
            ("# only\n", "no header"),
            ("artifact\ttest\n", "header"),
            ("artifact\tname\tubuntu-latest\n", "header"),
            ("artifact\ttest\tlocal\n", "unknown OS label"),
            ("artifact\ttest\twindows-latest\tubuntu-latest\n", "order"),
            ("artifact\ttest\tubuntu-latest\tubuntu-latest\n", "order"),
            (&format!("{HEADER}a/lib/a\tt\t1\n"), "cells"),
            (&format!("{HEADER}a/lib\tt\t1\t1\n"), "artifact key"),
            (&format!("{HEADER}a b/lib/a\tt\t1\t1\n"), "artifact key"),
            (&format!("{HEADER}a/lib/a\t t\t1\t1\n"), "test name"),
            (&format!("{HEADER}a/lib/a\t\t1\t1\n"), "test name"),
            (&format!("{HEADER}a/lib/a\tt\t-\t-\n"), "no weight"),
            (&format!("{HEADER}a/lib/a\tt\t01\t1\n"), "invalid weight"),
            (&format!("{HEADER}a/lib/a\tt\t-1\t1\n"), "invalid weight"),
            (&format!("{HEADER}a/lib/a\tt\t86400001\t1\n"), "exceeds"),
            (
                &format!("{HEADER}a/lib/a\tt\t99999999999999999999\t1\n"),
                "weight",
            ),
            (
                &format!("{HEADER}a/lib/a\tu\t1\t1\na/lib/a\tt\t1\t1\n"),
                "out of order",
            ),
            (
                &format!("{HEADER}a/lib/a\tt\t1\t1\na/lib/a\tt\t1\t1\n"),
                "repeated",
            ),
            (&format!("{HEADER}a/lib/a\tt\t1\t1\n\n"), "cells"),
            (&format!("{HEADER}# late\n"), "cells"),
        ] {
            let error = format!("{:#}", TimingTable::parse(text).unwrap_err());
            assert!(error.contains(reason), "{text:?}: {error}");
        }
    }

    fn owners(placement: &Placement, listed: &[(&str, &str)]) -> Vec<u32> {
        listed
            .iter()
            .map(|(artifact, test)| placement.owner(artifact, test))
            .collect()
    }

    #[test]
    fn placement_is_greedy_deterministic_and_independent_of_row_order() {
        let rows = [
            ("b/lib/b", "t::slow", 139),
            ("a/lib/a", "t::slow", 122),
            ("a/test/c", "t::one", 100),
            ("a/test/c", "t::two", 98),
            ("b/lib/b", "t::tie_b", 10),
            ("b/lib/b", "t::tie_a", 10),
            ("a/lib/a", "t::zero", 0),
        ];
        let placement = Placement::new(rows.iter().copied(), 3).unwrap();
        // Heaviest first to the least-loaded partition, ties to the lowest
        // index; equal weights go by artifact key, then test name.
        assert_eq!(placement.placed("b/lib/b", "t::slow"), Some(1));
        assert_eq!(placement.placed("a/lib/a", "t::slow"), Some(2));
        assert_eq!(placement.placed("a/test/c", "t::one"), Some(3));
        assert_eq!(placement.placed("a/test/c", "t::two"), Some(3));
        assert_eq!(placement.placed("b/lib/b", "t::tie_a"), Some(2));
        assert_eq!(placement.placed("b/lib/b", "t::tie_b"), Some(2));
        assert_eq!(placement.placed("a/lib/a", "t::zero"), Some(1));
        assert_eq!(placement.loads(), [139, 142, 198]);
        assert_eq!(placement.count(), 3);
        // Every permutation of the rows places them identically.
        let mut shuffled = rows.to_vec();
        for rotation in 0..shuffled.len() {
            shuffled.rotate_left(1);
            shuffled.swap(0, rotation);
            assert_eq!(
                Placement::new(shuffled.iter().copied(), 3).unwrap(),
                placement
            );
            shuffled.reverse();
            assert_eq!(
                Placement::new(shuffled.iter().copied(), 3).unwrap(),
                placement
            );
        }
        let error = Placement::new([("a/lib/a", "t", 1), ("a/lib/a", "t", 2)], 2).unwrap_err();
        assert!(error.to_string().contains("repeat"), "{error}");
        for count in [0, MAX_PARTITIONS + 1] {
            assert!(
                Placement::new(rows.iter().copied(), count).is_err(),
                "{count}"
            );
        }
    }

    #[test]
    fn every_test_has_one_owner_and_unknown_tests_keep_their_hash() {
        let rows: Vec<(String, String, u64)> = (0..200)
            .map(|index| {
                (
                    format!("p{}/lib/p", index % 5),
                    format!("t::case_{index:03}"),
                    (index * 7919 % 1000) as u64,
                )
            })
            .collect();
        let listed: Vec<(String, String)> = (0..260)
            .map(|index| {
                (
                    format!("p{}/lib/p", index % 5),
                    format!("t::case_{index:03}"),
                )
            })
            .collect();
        for count in 1..=16 {
            let placement = Placement::new(
                rows.iter().map(|(a, t, w)| (a.as_str(), t.as_str(), *w)),
                count,
            )
            .unwrap();
            let mut totals = vec![0_u64; count as usize];
            for (artifact, test) in &listed {
                let owner = placement.owner(artifact, test);
                assert!((1..=count).contains(&owner));
                match placement.placed(artifact, test) {
                    Some(placed) => assert_eq!(placed, owner),
                    None => assert_eq!(owner, partition::assign(artifact, test, count)),
                }
                if let Some((_, _, weight)) =
                    rows.iter().find(|(a, t, _)| a == artifact && t == test)
                {
                    totals[owner as usize - 1] += weight;
                }
            }
            assert_eq!(totals, placement.loads(), "{count}");
            // Greedy placement ends within the largest weight of the mean.
            let mean = totals.iter().sum::<u64>() / u64::from(count);
            assert!(
                totals.iter().all(|total| total.abs_diff(mean) <= 1000),
                "{totals:?}"
            );
        }
        // No label, no column: every test keeps its hash partition.
        let table = table("a/lib/a\tt::x\t5\t-\n");
        for host in ["x86_64-apple-darwin", "aarch64-apple-darwin"] {
            let placement = Placement::for_host(&table, host, 8).unwrap();
            assert_eq!(placement.placed("a/lib/a", "t::x"), None);
            assert_eq!(
                owners(&placement, &[("a/lib/a", "t::x")]),
                [partition::assign("a/lib/a", "t::x", 8)]
            );
        }
        let placement = Placement::for_host(&table, "x86_64-unknown-linux-gnu", 8).unwrap();
        assert_eq!(placement.placed("a/lib/a", "t::x"), Some(1));
        assert_eq!(
            label_for_host("x86_64-pc-windows-msvc"),
            Some("windows-latest")
        );
        assert_eq!(label_for_host("local"), None);
    }

    fn completed(tests: &[(&str, u64)]) -> Vec<CompletedTest> {
        tests
            .iter()
            .map(|(name, millis)| CompletedTest {
                name: (*name).to_owned(),
                millis: *millis,
            })
            .collect()
    }

    #[test]
    fn attribution_sums_to_the_last_completion() {
        let tests = completed(&[("c", 900), ("a", 100), ("b", 100), ("d", 950)]);
        let attributed = attribute(&tests);
        assert_eq!(attributed, [("a", 100), ("b", 0), ("c", 800), ("d", 50)]);
        assert_eq!(
            attributed.iter().map(|(_, weight)| weight).sum::<u64>(),
            950
        );
        assert!(attribute(&[]).is_empty());
    }

    #[test]
    fn refresh_replaces_measured_columns_and_keeps_the_rest() {
        let base = table("a/lib/a\tkept\t-\t4\na/lib/a\told\t5\t9\n");
        let key = |test: &str| ("a/lib/a".to_owned(), test.to_owned());
        let newest = Measured::from([(
            "ubuntu-latest".to_owned(),
            BTreeMap::from([(key("new"), 10), (key("kept"), 3)]),
        )]);
        let older = Measured::from([
            (
                "ubuntu-latest".to_owned(),
                BTreeMap::from([(key("new"), 21), (key("gone"), 50)]),
            ),
            ("macos-latest".to_owned(), BTreeMap::from([(key("new"), 1)])),
        ]);
        let refreshed = refresh(&base, &[newest, older]).unwrap();
        assert_eq!(refreshed.labels(), ["ubuntu-latest", "windows-latest"]);
        let rows: Vec<_> = refreshed
            .rows()
            .iter()
            .map(|row| (row.test.as_str(), row.weights.clone()))
            .collect();
        assert_eq!(
            rows,
            [
                ("kept", vec![Some(3), Some(4)]),
                // The mean of 10 and 21, rounded half up.
                ("new", vec![Some(16), None]),
                ("old", vec![None, Some(9)]),
            ]
        );
        assert!(refresh(&base, &[]).is_err());
        let over = Measured::from([(
            "ubuntu-latest".to_owned(),
            BTreeMap::from([(key("x"), MAX_WEIGHT_MS + 1)]),
        )]);
        assert!(refresh(&base, &[over]).is_err());
    }

    #[tokio::test]
    async fn run_evidence_measures_every_timed_test_and_regenerates_the_table() {
        use super::super::{Mode, fixture::Workspace, partition::PartitionScheme};

        let workspace = Workspace::new(Mode::Uninstrumented);
        let inputs = workspace.temp.path().join("inputs");
        fs::create_dir(&inputs).unwrap();
        assert!(format!("{:#}", measure(&inputs).unwrap_err()).contains("no partition evidence"));
        let name = |index: u32, attempt: u64| {
            inputs.join(format!(
                "ci-coverage-ubuntu-latest-partition-{index}-attempt-{attempt}"
            ))
        };
        for (index, attempt) in [(1, 1), (2, 1), (1, 2)] {
            let partition = PartitionScheme::new(index, 2).unwrap();
            workspace
                .evidence(
                    Mode::Uninstrumented,
                    &partition,
                    attempt,
                    &name(index, attempt),
                )
                .await
                .unwrap();
        }
        // Other downloaded artifacts and files are not partition evidence.
        fs::create_dir_all(inputs.join("ci-coverage-ubuntu-latest-attempt-1")).unwrap();
        fs::write(
            inputs.join("ci-coverage-ubuntu-latest-attempt-1/coverage.lcov"),
            b"",
        )
        .unwrap();
        fs::write(inputs.join("notes.txt"), b"").unwrap();

        let measured = measure(&inputs).unwrap();
        assert_eq!(measured.keys().collect::<Vec<_>>(), ["ubuntu-latest"]);
        let tests = &measured["ubuntu-latest"];
        let listed: usize = workspace
            .lists
            .values()
            .map(|text| text.lines().count())
            .sum();
        // Each scripted completion follows the previous one by 1 ms, so
        // every selected test is attributed 1 ms, once, across partitions.
        assert_eq!(tests.len(), listed);
        assert!(tests.values().all(|weight| *weight == 1), "{tests:?}");

        let table = workspace.temp.path().join("timings.tsv");
        fs::write(&table, "artifact\ttest\twindows-latest\na/lib/a\tt\t5\n").unwrap();
        let described = regenerate(&table, std::slice::from_ref(&inputs)).unwrap();
        assert!(
            described.contains("ubuntu-latest (8 partitions,"),
            "{described}"
        );
        assert!(
            described.contains("windows-latest (8 partitions, 1 rows)"),
            "{described}"
        );
        let refreshed = TimingTable::parse(&fs::read_to_string(&table).unwrap()).unwrap();
        assert_eq!(refreshed.labels(), ["ubuntu-latest", "windows-latest"]);
        assert_eq!(refreshed.weights("ubuntu-latest").len(), listed);
        assert_eq!(refreshed.weights("windows-latest"), [("a/lib/a", "t", 5)]);
        assert!(
            regenerate(
                &workspace.temp.path().join("missing.tsv"),
                std::slice::from_ref(&inputs)
            )
            .is_err()
        );

        // A second copy of one attempt, and a ledger that differs from its
        // receipt, are refused.
        let copy = inputs.join("ci-coverage-ubuntu-latest-partition-2-attempt-1-copy");
        fs::create_dir_all(copy.join("attempt-1")).unwrap();
        for entry in fs::read_dir(name(2, 1).join("attempt-1")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), copy.join("attempt-1").join(entry.file_name())).unwrap();
        }
        assert!(format!("{:#}", measure(&inputs).unwrap_err()).contains("appears twice"));
        fs::remove_dir_all(&copy).unwrap();

        // A label missing a partition, or evidence mixing sources, trees,
        // partition counts or out-of-range indexes, is refused, because the
        // refresh would replace the whole column with part of a run.
        let aside = workspace.temp.path().join("aside");
        fs::rename(name(2, 1), &aside).unwrap();
        let error = format!("{:#}", measure(&inputs).unwrap_err());
        assert!(
            error.contains("ubuntu-latest evidence lacks partitions [2] of 2"),
            "{error}"
        );
        fs::rename(&aside, name(2, 1)).unwrap();
        let receipt = name(2, 1).join("attempt-1").join(RECEIPT_FILE);
        let original = fs::read(&receipt).unwrap();
        type Edit = fn(&mut serde_json::Value);
        let edits: [(Edit, &str); 4] = [
            (
                |value| value["source"] = "0".repeat(40).into(),
                "give one run's evidence per --inputs directory",
            ),
            (
                |value| value["tree"] = "0".repeat(40).into(),
                "give one run's evidence per --inputs directory",
            ),
            (
                |value| value["partition"]["count"] = 3.into(),
                "names partition counts 2 and 3",
            ),
            (
                |value| value["partition"]["index"] = 3.into(),
                "names partition 3 of 2",
            ),
        ];
        for (edit, message) in edits {
            let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
            edit(&mut value);
            fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
            let error = format!("{:#}", measure(&inputs).unwrap_err());
            assert!(error.contains(message), "{message}: {error}");
        }
        fs::write(&receipt, &original).unwrap();
        measure(&inputs).unwrap();
        let ledger = name(2, 1).join("attempt-1").join(RUNNER_LEDGER_FILE);
        let mut bytes = fs::read(&ledger).unwrap();
        bytes.push(b'\n');
        fs::write(&ledger, bytes).unwrap();
        assert!(
            format!("{:#}", measure(&inputs).unwrap_err()).contains("differs from its receipt")
        );
    }

    fn plan(index: u32, executables: &[(&str, &[&str], &[&str], bool)]) -> PartitionPlan {
        PartitionPlan {
            schema: super::super::SCHEMA,
            inventory_sha256: String::new(),
            partition: partition::PartitionScheme::new(index, 2).unwrap(),
            executables: executables
                .iter()
                .map(
                    |(artifact, listed, assigned, excluded)| plan::ExecutableTests {
                        artifact: (*artifact).to_owned(),
                        executable: String::new(),
                        listed: listed.iter().map(|name| (*name).to_owned()).collect(),
                        list_sha256: String::new(),
                        assigned: assigned.iter().map(|name| (*name).to_owned()).collect(),
                        assigned_sha256: String::new(),
                        excluded: excluded.then(|| "reason".to_owned()),
                    },
                )
                .collect(),
        }
    }

    #[test]
    fn reports_predict_partitions_and_warn_on_unknown_tests() {
        let table = table("a/lib/a\tgone\t6000\t-\na/lib/a\tx\t4000\t-\na/lib/a\ty\t2000\t-\n");
        let first = plan(
            1,
            &[
                ("a/lib/a", &["new", "x", "y"], &["x"], false),
                ("e/lib/e", &["z"], &[], true),
            ],
        );
        let second = plan(
            2,
            &[
                ("a/lib/a", &["new", "x", "y"], &["new", "y"], false),
                ("e/lib/e", &["z"], &[], true),
            ],
        );
        let plans = [(1, &first), (2, &second)];
        let report = report(&table, "abc", "x86_64-unknown-linux-gnu", 2, &plans);
        assert_eq!(report.label.as_deref(), Some("ubuntu-latest"));
        assert_eq!(report.default_weight_ms, 4000);
        assert_eq!(report.listed, 3);
        assert_eq!(report.unknown, ["a/lib/a new"]);
        assert_eq!(report.stale_rows, 1);
        assert_eq!(report.predicted_ms, [4000, 6000]);
        let warning = report.warning.clone().unwrap();
        assert!(warning.contains("1 of 3 listed tests"), "{warning}");
        let markdown = report.markdown("ubuntu-latest", &[Some(5), None]);
        assert!(markdown.contains("| 1 | 4.0 | 5 |"), "{markdown}");
        assert!(markdown.contains("| 2 | 6.0 | - |"), "{markdown}");
        assert!(markdown.contains("[!WARNING]"), "{markdown}");
        assert!(markdown.contains("- `a/lib/a new`"), "{markdown}");

        // Within the threshold: no warning. A host without a label counts
        // every test as unknown.
        let known = plan(1, &[("a/lib/a", &["x", "y"], &["x", "y"], false)]);
        let quiet = super::report(&table, "abc", "x86_64-unknown-linux-gnu", 1, &[(1, &known)]);
        assert_eq!(quiet.warning, None);
        assert!(!quiet.markdown("ubuntu-latest", &[]).contains("WARNING"));
        let local = super::report(
            &table,
            "abc",
            "riscv64gc-unknown-linux-gnu",
            1,
            &[(1, &known)],
        );
        assert_eq!(local.label, None);
        assert_eq!(local.unknown.len(), 2);
        assert_eq!(local.predicted_ms, [2 * FALLBACK_WEIGHT_MS]);
        let many: Vec<String> = (0..60).map(|index| format!("n{index:02}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        let wide = plan(1, &[("a/lib/a", &names, &names, false)]);
        let long = super::report(&table, "abc", "x86_64-unknown-linux-gnu", 1, &[(1, &wide)]);
        assert!(
            long.markdown("ubuntu-latest", &[])
                .contains("- and 10 more")
        );
        assert_eq!(seconds(12_345), "12.3");
    }
}
