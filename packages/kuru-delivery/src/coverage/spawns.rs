//! Spawn rows of the runner ledger, and the attribution of a raw profile to
//! the process and test that wrote it.
//!
//! Every instrumented process writes `<prefix>-<pid>-<signature>[_<pool>].profraw`
//! (`LLVM_PROFILE_FILE`'s `%p` and `%m`). The module signature is computed per
//! executable, so it names a binary, not a test. Test support appends one
//! spawn row to the runner ledger for each child that leaves the test's
//! process group (memory supervisors and owners, terminal children), and the
//! runner appends one for each test executable's own listing process and each
//! Windows selection's actual test-run PID. A
//! profile that appears after the partition's tests is then named by its
//! process ID: the test that started it, its role and its executable; its
//! signature is named by every other profile of the same executable.
//!
//! Spawn rows are informational, like the job ledger: they never change a
//! plan, and a row from a later run of a reused process ID is reported as
//! such rather than trusted.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// The environment variable test support reads to find the runner ledger.
/// `kuru-memory`'s `test_support::spawn_ledger` defines the same name.
pub const SPAWN_LEDGER_ENV: &str = "KURU_COVERAGE_SPAWN_LEDGER";
/// The `record` value of a spawn row.
pub const SPAWN_RECORD: &str = "spawn";
/// The spawn row format version this parser accepts.
pub const SPAWN_ROW_VERSION: u32 = 1;
/// Role of a test executable's own listing process, recorded by the runner.
pub const TEST_EXECUTABLE: &str = "test-executable";

/// One process started during the partition's tests.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnRecord {
    /// Always [`SPAWN_RECORD`].
    pub record: String,
    pub version: u32,
    pub pid: u32,
    /// The process that started it.
    pub parent: u32,
    pub role: String,
    /// The started executable, as its spawner named it.
    pub executable: String,
    /// The originating test (its thread name), or the runner's artifact key.
    pub test: String,
    /// Unix seconds at the spawn.
    pub at: u64,
}

impl SpawnRecord {
    pub fn new(pid: u32, role: &str, executable: &str, test: &str, at: u64) -> Self {
        Self {
            record: SPAWN_RECORD.to_owned(),
            version: SPAWN_ROW_VERSION,
            pid,
            parent: std::process::id(),
            role: role.to_owned(),
            executable: executable.to_owned(),
            test: test.to_owned(),
            at,
        }
    }

    /// Parse one ledger line already known to carry `"record": "spawn"`.
    pub fn parse(value: serde_json::Value) -> Result<Self> {
        let record: Self = serde_json::from_value(value)?;
        ensure!(
            record.record == SPAWN_RECORD,
            "spawn row has record {:?}",
            record.record
        );
        ensure!(
            record.version == SPAWN_ROW_VERSION,
            "unsupported spawn row version {}",
            record.version
        );
        Ok(record)
    }

    /// Whether a ledger line is a spawn row rather than a runner record.
    pub fn is_spawn_row(value: &serde_json::Value) -> bool {
        value.get("record").and_then(serde_json::Value::as_str) == Some(SPAWN_RECORD)
    }
}

/// The process ID and module signature in a raw profile's name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileName {
    pub pid: u32,
    pub signature: String,
}

impl ProfileName {
    /// `<prefix>-<pid>-<signature>[_<pool>].profraw`, where the prefix may
    /// itself contain hyphens. `None` for any other name.
    pub fn parse(name: &str) -> Option<Self> {
        let stem = name.strip_suffix(".profraw")?;
        let (rest, module) = stem.rsplit_once('-')?;
        let (_, pid) = rest.rsplit_once('-')?;
        let signature = module
            .split_once('_')
            .map_or(module, |(signature, _)| signature);
        (!signature.is_empty() && signature.bytes().all(|byte| byte.is_ascii_digit()))
            .then_some(())?;
        Some(Self {
            pid: pid.parse().ok()?,
            signature: signature.to_owned(),
        })
    }
}

/// The executable a spawn row names, relative to the coverage target when it
/// lies beneath it.
fn shown(executable: &str, target: &Path) -> String {
    Path::new(executable)
        .strip_prefix(target)
        .map_or_else(|_| executable.to_owned(), |path| path.display().to_string())
}

/// The recorded processes above `row`, nearest first: a child that a
/// recorded child started (an owner's supervisor, a terminal child's owner)
/// names the test of the process that started it.
fn ancestry(row: &SpawnRecord, by_pid: &BTreeMap<u32, Vec<&SpawnRecord>>, target: &Path) -> String {
    const DEPTH: usize = 8;
    let mut text = String::new();
    let mut parent = row.parent;
    let mut seen = BTreeSet::from([row.pid]);
    for _ in 0..DEPTH {
        let Some([above]) = by_pid.get(&parent).map(Vec::as_slice) else {
            break;
        };
        if !seen.insert(above.pid) {
            break;
        }
        text.push_str(&format!(
            ", the {} {} started by test {}",
            above.role,
            shown(&above.executable, target),
            above.test
        ));
        parent = above.parent;
    }
    text
}

/// Attribute each changed profile, `(change, name)` such as `("new",
/// "kuru-1-2_0.profraw")`, to the process and test that wrote it.
/// `profiles` is every profile name now in the target root.
pub fn attribute(
    changes: &[(&str, &str)],
    profiles: &[&str],
    spawns: &[SpawnRecord],
    target: &Path,
) -> Vec<String> {
    let mut by_pid: BTreeMap<u32, Vec<&SpawnRecord>> = BTreeMap::new();
    for spawn in spawns {
        by_pid.entry(spawn.pid).or_default().push(spawn);
    }
    // A signature names the executables of every recorded process that wrote
    // a profile with it.
    let mut executables: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for profile in profiles.iter().filter_map(|name| ProfileName::parse(name)) {
        if let Some(rows) = by_pid.get(&profile.pid) {
            for row in rows {
                executables
                    .entry(profile.signature.clone())
                    .or_default()
                    .insert(shown(&row.executable, target));
            }
        }
    }
    changes
        .iter()
        .map(|(change, name)| {
            let Some(profile) = ProfileName::parse(name) else {
                return format!("{change} {name} (not a %p-%m profile name)");
            };
            let process = match by_pid.get(&profile.pid).map(Vec::as_slice) {
                None | Some([]) => format!(
                    "pid {} has no spawn row: it was not started by recorded test support",
                    profile.pid
                ),
                Some([row]) => format!(
                    "pid {} is the {} {} started by test {} (parent pid {}{})",
                    profile.pid,
                    row.role,
                    shown(&row.executable, target),
                    row.test,
                    row.parent,
                    ancestry(row, &by_pid, target)
                ),
                Some(rows) => format!(
                    "pid {} was reused; its spawn rows name {}",
                    profile.pid,
                    rows.iter()
                        .map(|row| format!(
                            "the {} {} started by test {} at {}",
                            row.role,
                            shown(&row.executable, target),
                            row.test,
                            row.at
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            };
            let binary = executables.get(&profile.signature).map_or_else(
                || format!("no recorded process shares signature {}", profile.signature),
                |names| {
                    format!(
                        "signature {} is {}",
                        profile.signature,
                        names.iter().cloned().collect::<Vec<_>>().join(" or ")
                    )
                },
            );
            format!("{change} {name}: {process}; {binary}")
        })
        .collect()
}

/// Profile names directly in the target root, sorted.
pub fn profile_names(target: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(target)? {
        let path = entry?.path();
        if path.extension() == Some(std::ffi::OsStr::new("profraw")) {
            names.push(
                path.file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .with_context(|| format!("profile {} lacks a UTF-8 name", path.display()))?
                    .to_owned(),
            );
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, role: &str, executable: &str, test: &str) -> SpawnRecord {
        SpawnRecord::new(pid, role, executable, test, 1)
    }

    #[test]
    fn profile_names_yield_the_pid_and_signature_under_any_prefix() {
        assert_eq!(
            ProfileName::parse("kuru-13664-10634524661574408682_0.profraw"),
            Some(ProfileName {
                pid: 13664,
                signature: "10634524661574408682".into()
            })
        );
        assert_eq!(
            ProfileName::parse("fix-a-b-7-42.profraw"),
            Some(ProfileName {
                pid: 7,
                signature: "42".into()
            })
        );
        for name in [
            "kuru-late-1.profraw",
            "kuru-1-2.profdata",
            "kuru-x-2_0.profraw",
            "kuru-1-_0.profraw",
            "nohyphen.profraw",
        ] {
            assert_eq!(ProfileName::parse(name), None, "{name}");
        }
    }

    #[test]
    fn a_late_profile_names_its_test_role_and_executable_by_pid_and_signature() {
        let target = Path::new("/target");
        let spawns = [
            row(
                10,
                TEST_EXECUTABLE,
                "/target/debug/deps/kuru_runtime-1",
                "kuru-runtime/lib/kuru_runtime",
            ),
            row(
                11,
                "dolt-supervisor",
                "/target/debug/kuru-memory",
                "tests::earlier",
            ),
            row(
                12,
                "dolt-supervisor",
                "/target/debug/kuru-memory",
                "tests::last",
            ),
        ];
        let profiles = [
            "kuru-10-1_0.profraw",
            "kuru-11-2_0.profraw",
            "kuru-12-2_0.profraw",
            "kuru-13-2_0.profraw",
            "kuru-14-3_0.profraw",
        ];
        let lines = attribute(
            &[
                ("new", "kuru-12-2_0.profraw"),
                ("new", "kuru-13-2_0.profraw"),
                ("changed", "kuru-14-3_0.profraw"),
                ("removed", "odd.profraw"),
            ],
            &profiles,
            &spawns,
            target,
        );
        assert_eq!(
            lines,
            [
                "new kuru-12-2_0.profraw: pid 12 is the dolt-supervisor debug/kuru-memory started \
                 by test tests::last (parent pid "
                    .to_owned()
                    + &std::process::id().to_string()
                    + "); signature 2 is debug/kuru-memory",
                "new kuru-13-2_0.profraw: pid 13 has no spawn row: it was not started by \
                 recorded test support; signature 2 is debug/kuru-memory"
                    .to_owned(),
                "changed kuru-14-3_0.profraw: pid 14 has no spawn row: it was not started by \
                 recorded test support; no recorded process shares signature 3"
                    .to_owned(),
                "removed odd.profraw (not a %p-%m profile name)".to_owned(),
            ]
        );
    }

    #[test]
    fn a_grandchild_names_the_recorded_processes_above_it() {
        let mut terminal = row(20, "terminal-child", "/target/debug/kuru", "tests::pty");
        terminal.parent = 1;
        let mut owner = row(21, "memory-owner", "/target/debug/kuru", "main");
        owner.parent = 20;
        let mut supervisor = row(22, "dolt-supervisor", "/target/debug/kuru", "main");
        supervisor.parent = 21;
        let lines = attribute(
            &[("new", "kuru-22-4_0.profraw")],
            &["kuru-22-4_0.profraw"],
            &[terminal, owner, supervisor],
            Path::new("/target"),
        );
        assert_eq!(
            lines,
            [
                "new kuru-22-4_0.profraw: pid 22 is the dolt-supervisor debug/kuru started by test \
              main (parent pid 21, the memory-owner debug/kuru started by test main, the \
              terminal-child debug/kuru started by test tests::pty); signature 4 is debug/kuru"
            ]
        );
    }

    #[test]
    fn a_reused_pid_names_every_row_instead_of_choosing_one() {
        let spawns = [
            row(5, "memory-owner", "/elsewhere/kuru", "tests::a"),
            row(5, "terminal-child", "/elsewhere/kuru", "tests::b"),
        ];
        let lines = attribute(
            &[("new", "kuru-5-9_0.profraw")],
            &["kuru-5-9_0.profraw"],
            &spawns,
            Path::new("/target"),
        );
        assert_eq!(
            lines,
            [
                "new kuru-5-9_0.profraw: pid 5 was reused; its spawn rows name the memory-owner \
              /elsewhere/kuru started by test tests::a at 1; the terminal-child /elsewhere/kuru \
              started by test tests::b at 1; signature 9 is /elsewhere/kuru"
            ]
        );
    }

    #[test]
    fn spawn_rows_are_recognised_and_parsed_strictly() {
        let value = serde_json::to_value(row(1, "dolt-supervisor", "/x", "t")).unwrap();
        assert!(SpawnRecord::is_spawn_row(&value));
        assert_eq!(
            SpawnRecord::parse(value.clone()).unwrap(),
            row(1, "dolt-supervisor", "/x", "t")
        );
        let mut extra = value.clone();
        extra["unknown"] = 1.into();
        assert!(SpawnRecord::parse(extra).is_err());
        let mut newer = value;
        newer["version"] = 2.into();
        assert!(SpawnRecord::parse(newer).is_err());
        assert!(!SpawnRecord::is_spawn_row(
            &serde_json::json!({"schema": 2})
        ));
    }
}
