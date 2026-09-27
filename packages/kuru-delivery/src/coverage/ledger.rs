//! The informational job ledger of one partition.
//!
//! It records phase timestamps, cache state, dependency units Cargo rebuilt,
//! the profile total and per-executable timings, so the effect of warm or
//! evicted caches is visible per job. It is hashed into the receipt for
//! integrity but never compared across partitions, and nothing in it can fail
//! a run except a malformed file the partition itself wrote.

use super::{Mode, PROFILE_COUNT_LIMIT, SCHEMA, partition::PartitionScheme, plan::RunnerRecord};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

/// Start and end of one phase, in Unix seconds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub started: u64,
    pub finished: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    pub entries: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheLedger {
    /// The helper build cache: `hit`, `miss` or `unknown`.
    pub helper: String,
    /// The dependency seed: `hit`, `partial`, `miss`, `absent` or `unknown`.
    pub seed: String,
    pub seed_matched_key: Option<String>,
    pub seed_imported: Transfer,
    /// Refused seed entries by reason.
    pub seed_refused: BTreeMap<String, u64>,
    /// Non-workspace and workspace units Cargo compiled rather than reused.
    pub dependency_units_rebuilt: u64,
    pub workspace_units_rebuilt: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileLedger {
    pub count: usize,
    pub bytes: u64,
    pub limit: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableLedger {
    pub artifact: String,
    pub started: u64,
    pub finished: u64,
    pub invocations: usize,
    /// Raw profiles this executable's processes added.
    pub profraw: usize,
    pub listed: usize,
    pub assigned: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobLedger {
    pub schema: u32,
    pub partition: PartitionScheme,
    pub mode: Mode,
    pub job_started: u64,
    pub phases: BTreeMap<String, Phase>,
    pub cache: CacheLedger,
    pub profiles: ProfileLedger,
    pub executables: Vec<ExecutableLedger>,
}

impl JobLedger {
    /// Parse and check a ledger's internal consistency.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let ledger: Self = serde_json::from_slice(bytes).context("parse the job ledger")?;
        ensure!(ledger.schema == SCHEMA, "unsupported job ledger schema");
        ledger.partition.validate()?;
        for (name, phase) in &ledger.phases {
            ensure!(
                phase.started <= phase.finished,
                "job ledger phase {name} finished before it started"
            );
        }
        for executable in &ledger.executables {
            ensure!(
                executable.started <= executable.finished,
                "job ledger executable {} finished before it started",
                executable.artifact
            );
        }
        Ok(ledger)
    }

    /// Seconds spent in a phase, if it was recorded.
    pub fn seconds(&self, phase: &str) -> Option<u64> {
        self.phases
            .get(phase)
            .map(|phase| phase.finished - phase.started)
    }

    /// The executable with the longest wall time.
    pub fn slowest(&self) -> Option<(&str, u64)> {
        self.executables
            .iter()
            .map(|executable| {
                (
                    executable.artifact.as_str(),
                    executable.finished - executable.started,
                )
            })
            .max_by_key(|(artifact, seconds)| (*seconds, std::cmp::Reverse(*artifact)))
    }
}

/// A `cache-hit` step output: `true` is a hit, `false` a miss, anything else
/// (including an absent or empty value) is unknown and never an error.
pub fn cache_state(value: Option<&str>) -> &'static str {
    match value.map(str::trim) {
        Some("true") => "hit",
        Some("false") => "miss",
        _ => "unknown",
    }
}

/// The dependency seed state from its configuration and restore outputs.
pub fn seed_state(configured: bool, restored: bool, cache_hit: Option<&str>) -> &'static str {
    if !configured {
        return "absent";
    }
    if !restored {
        return "miss";
    }
    match cache_state(cache_hit) {
        "hit" => "hit",
        // Restored without an exact hit: a prefix restore key matched.
        "miss" => "partial",
        _ => "unknown",
    }
}

/// Units Cargo compiled rather than reused, split by workspace membership,
/// from the `compiler-artifact` messages' `fresh` flags.
pub fn rebuilt_units(messages: &Path, workspace_ids: &BTreeSet<String>) -> Result<(u64, u64)> {
    #[derive(Deserialize)]
    struct Message {
        reason: String,
        package_id: Option<String>,
        fresh: Option<bool>,
    }

    let mut dependencies = 0;
    let mut workspace = 0;
    let reader = BufReader::new(File::open(messages)?);
    for (index, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("read Cargo message line {}", index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Message = serde_json::from_str(&line)
            .with_context(|| format!("parse Cargo message line {}", index + 1))?;
        if message.reason != "compiler-artifact" || message.fresh != Some(false) {
            continue;
        }
        match message.package_id {
            Some(id) if workspace_ids.contains(&id) => workspace += 1,
            _ => dependencies += 1,
        }
    }
    Ok((dependencies, workspace))
}

/// Per-executable timings and profile counts from the runner ledger.
pub fn executables(records: &[RunnerRecord]) -> Vec<ExecutableLedger> {
    let mut executables: Vec<_> = records
        .iter()
        .map(|record| ExecutableLedger {
            artifact: record.artifact.clone(),
            started: record.started,
            finished: record.finished.max(record.started),
            invocations: record.invocations.len(),
            profraw: record.profraw_after.saturating_sub(record.profraw_before),
            listed: record.listed.len(),
            assigned: record.assigned,
        })
        .collect();
    executables.sort_by(|left, right| left.artifact.cmp(&right.artifact));
    executables
}

/// The profile total of a partition, with the per-partition count limit.
pub fn profiles(count: usize, bytes: u64) -> ProfileLedger {
    ProfileLedger {
        count,
        bytes,
        limit: PROFILE_COUNT_LIMIT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> JobLedger {
        JobLedger {
            schema: SCHEMA,
            partition: PartitionScheme::new(2, 4).unwrap(),
            mode: Mode::Instrumented,
            job_started: 10,
            phases: BTreeMap::from([
                (
                    "compile".to_owned(),
                    Phase {
                        started: 20,
                        finished: 80,
                    },
                ),
                (
                    "tests".to_owned(),
                    Phase {
                        started: 80,
                        finished: 200,
                    },
                ),
            ]),
            cache: CacheLedger {
                helper: cache_state(Some("true")).to_owned(),
                seed: seed_state(true, true, Some("false")).to_owned(),
                seed_matched_key: Some("kuru-coverage-seed-v1-x-".to_owned()),
                seed_imported: Transfer {
                    entries: 3,
                    bytes: 30,
                },
                seed_refused: BTreeMap::from([("workspace".to_owned(), 2)]),
                dependency_units_rebuilt: 0,
                workspace_units_rebuilt: 12,
            },
            profiles: profiles(5, 500),
            executables: vec![
                ExecutableLedger {
                    artifact: "b/lib/b".to_owned(),
                    started: 80,
                    finished: 150,
                    invocations: 2,
                    profraw: 2,
                    listed: 4,
                    assigned: 1,
                },
                ExecutableLedger {
                    artifact: "a/lib/a".to_owned(),
                    started: 150,
                    finished: 220,
                    invocations: 2,
                    profraw: 2,
                    listed: 4,
                    assigned: 1,
                },
            ],
        }
    }

    #[test]
    fn ledgers_round_trip_and_report_phases() {
        let ledger = ledger();
        let bytes = serde_json::to_vec(&ledger).unwrap();
        let parsed = JobLedger::parse(&bytes).unwrap();
        assert_eq!(parsed, ledger);
        assert_eq!(parsed.seconds("compile"), Some(60));
        assert_eq!(parsed.seconds("seed_import"), None);
        // Ties prefer the lexically first artifact.
        assert_eq!(parsed.slowest(), Some(("a/lib/a", 70)));
        assert_eq!(parsed.cache.helper, "hit");
        assert_eq!(parsed.cache.seed, "partial");
        assert_eq!(parsed.profiles.limit, PROFILE_COUNT_LIMIT);
    }

    #[test]
    fn malformed_ledgers_are_refused() {
        let mut backwards = ledger();
        backwards.phases.get_mut("tests").unwrap().finished = 1;
        let mut executable = ledger();
        executable.executables[0].finished = 0;
        let mut schema = ledger();
        schema.schema = 1;
        for (ledger, reason) in [
            (backwards, "phase tests"),
            (executable, "executable b/lib/b"),
            (schema, "schema"),
        ] {
            let error = JobLedger::parse(&serde_json::to_vec(&ledger).unwrap())
                .unwrap_err()
                .to_string();
            assert!(error.contains(reason), "{error}");
        }
        assert!(JobLedger::parse(b"{}").is_err());
    }

    #[test]
    fn empty_cache_inputs_record_unknown_without_failing() {
        assert_eq!(cache_state(None), "unknown");
        assert_eq!(cache_state(Some("")), "unknown");
        assert_eq!(cache_state(Some(" false ")), "miss");
        assert_eq!(cache_state(Some("yes")), "unknown");
        assert_eq!(seed_state(false, false, None), "absent");
        assert_eq!(seed_state(true, false, Some("true")), "miss");
        assert_eq!(seed_state(true, true, Some("true")), "hit");
        assert_eq!(seed_state(true, true, None), "unknown");
    }

    #[test]
    fn rebuilt_units_count_only_unfresh_compiler_artifacts() {
        let temp = tempfile::TempDir::new().unwrap();
        let messages = temp.path().join("messages.json");
        std::fs::write(
            &messages,
            [
                r#"{"reason":"compiler-artifact","package_id":"dep","fresh":false}"#,
                r#"{"reason":"compiler-artifact","package_id":"dep2","fresh":true}"#,
                r#"{"reason":"compiler-artifact","package_id":"ws","fresh":false}"#,
                r#"{"reason":"build-script-executed","package_id":"dep3"}"#,
                "",
                r#"{"reason":"build-finished","success":true}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let workspace = BTreeSet::from(["ws".to_owned()]);
        assert_eq!(rebuilt_units(&messages, &workspace).unwrap(), (1, 1));
        std::fs::write(&messages, "not json\n").unwrap();
        assert!(rebuilt_units(&messages, &workspace).is_err());
    }
}
