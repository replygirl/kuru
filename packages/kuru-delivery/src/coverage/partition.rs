//! Deterministic per-test partitions of libtest executables.
//!
//! Every partition lists every executable's tests and assigns each listed test
//! to exactly one partition: the checked-in timing table's placement for the
//! host's OS label (see [`super::timing`]) or, for a test without a row, a
//! hash of its artifact identity and name. It runs its own tests with explicit
//! `--exact` selections. Selections are chunked under one Windows command-line
//! budget on every OS, so chunk boundaries are identical everywhere and
//! testable on any host.

use super::{
    Artifact,
    timing::{self, Placement},
};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// The assignment function's name, recorded in every plan and receipt.
pub const SCHEME: &str = "timed-lpt-v1";
/// The hash domain of a test without a timing row. It is the previous scheme's
/// name, so such a test keeps the partition it had before timing placement.
const HASH_DOMAIN: &str = "sha256-artifact-test-v1";
/// The largest partition count any OS may use.
pub const MAX_PARTITIONS: u32 = 64;
/// UTF-16 units allowed per exact-selection command line. Windows
/// `CreateProcess` accepts 32,767 including the terminator; the rest is margin.
pub const COMMAND_LINE_BUDGET: usize = 30_000;

/// One partition of one OS's partition set.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionScheme {
    pub scheme: String,
    /// One-based index, `1..=count`.
    pub index: u32,
    pub count: u32,
    /// Canonical digest of the timing table that places the tests.
    pub timings_sha256: String,
}

impl PartitionScheme {
    pub fn new(index: u32, count: u32) -> Result<Self> {
        let scheme = Self {
            scheme: SCHEME.to_owned(),
            index,
            count,
            timings_sha256: timing::embedded_sha256()?.to_owned(),
        };
        scheme.validate()?;
        Ok(scheme)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.scheme == SCHEME,
            "unknown partition scheme {:?}",
            self.scheme
        );
        ensure!(
            (1..=MAX_PARTITIONS).contains(&self.count),
            "partition count {} is outside 1..={MAX_PARTITIONS}",
            self.count
        );
        ensure!(
            (1..=self.count).contains(&self.index),
            "partition index {} is outside 1..={}",
            self.index,
            self.count
        );
        let table = timing::embedded_sha256()?;
        ensure!(
            self.timings_sha256 == table,
            "partition {} of {} was placed with timing table {}, but this helper's {} is {table}",
            self.index,
            self.count,
            self.timings_sha256,
            timing::TABLE_PATH
        );
        Ok(())
    }

    /// The checked-in table's placement for a host target at this count.
    pub fn placement(&self, host: &str) -> Result<Placement> {
        self.validate()?;
        Placement::for_host(timing::embedded()?, host, self.count)
    }

    /// This partition's names from a sorted list, in list order.
    pub fn assigned(
        &self,
        placement: &Placement,
        key: &str,
        listed: &[String],
    ) -> Result<Vec<String>> {
        ensure!(
            placement.count() == self.count,
            "a placement for {} partitions cannot assign partition {} of {}",
            placement.count(),
            self.index,
            self.count
        );
        Ok(listed
            .iter()
            .filter(|name| placement.owner(key, name) == self.index)
            .cloned()
            .collect())
    }
}

/// Stable identity of one libtest executable, independent of Cargo's metadata
/// hash and the OS executable suffix: `package/kind/target`.
pub fn artifact_key(artifact: &Artifact) -> String {
    format!(
        "{}/{}/{}",
        artifact.package,
        artifact.target_kind.join(","),
        artifact.target_name
    )
}

/// The one-based hash partition of a test without a timing row.
pub fn assign(key: &str, name: &str, count: u32) -> u32 {
    let mut hasher = Sha256::new();
    hasher.update(HASH_DOMAIN.as_bytes());
    hasher.update([0]);
    hasher.update(key.as_bytes());
    hasher.update([0]);
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    // The remainder is below `count`, which is a `u32`.
    1 + (u64::from_be_bytes(prefix) % u64::from(count.max(1))) as u32
}

/// Parse `<exe> --list --format terse` exactly: one `<name>: test` line per
/// test, nothing else. Returns the names sorted; an empty list is valid.
pub fn parse_libtest_list(text: &str) -> Result<Vec<String>> {
    let mut names = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some(name) = line.strip_suffix(": test") else {
            if line.ends_with(": bench") {
                bail!("libtest list line {number} names a benchmark: {line:?}");
            }
            bail!("unexpected libtest list line {number}: {line:?}");
        };
        ensure!(
            !name.is_empty() && name.trim() == name,
            "libtest list line {number} has an empty or padded name: {line:?}"
        );
        ensure!(
            !name.chars().any(char::is_control),
            "libtest list line {number} names a test with a control character"
        );
        ensure!(
            names.insert(name.to_owned()),
            "libtest lists test {name:?} more than once"
        );
    }
    Ok(names.into_iter().collect())
}

/// Digest of a name list in its canonical form: each name followed by `\n`.
pub fn list_sha256(names: &[String]) -> String {
    let mut hasher = Sha256::new();
    for name in names {
        hasher.update(name.as_bytes());
        hasher.update(b"\n");
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// UTF-16 units of one argument as `kuru_platform`'s Windows process
/// launcher writes it: every argument is quoted, `n` backslashes before a
/// literal quote become `2n + 1`, and `n` trailing backslashes become `2n`.
/// Counting any argument unquoted would undercount the spawned command line.
fn argument_units(argument: &str) -> usize {
    let mut units = 0;
    let mut backslashes = 0;
    for character in argument.chars() {
        if character == '\\' {
            backslashes += 1;
        } else {
            if character == '"' {
                units += backslashes + 1;
            }
            backslashes = 0;
        }
        units += character.len_utf16();
    }
    units + backslashes + 2
}

/// UTF-16 units of the complete command line the platform launcher builds,
/// including separators and the terminator.
pub fn windows_command_line_units(program: &str, args: &[&str]) -> usize {
    argument_units(program)
        + args
            .iter()
            .map(|argument| 1 + argument_units(argument))
            .sum::<usize>()
        + 1
}

/// Split names into consecutive chunks whose command lines, each
/// `program fixed... names...`, stay within `budget`. The chunks' union is the
/// input in order. A name that cannot fit alone fails.
pub fn chunk(
    program: &str,
    fixed: &[&str],
    names: &[String],
    budget: usize,
) -> Result<Vec<Vec<String>>> {
    let base = windows_command_line_units(program, fixed);
    let mut chunks = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut used = base;
    for name in names {
        let units = 1 + argument_units(name);
        ensure!(
            base + units <= budget,
            "test {name:?} does not fit a {budget}-unit command line with {program}"
        );
        if used + units > budget {
            chunks.push(std::mem::take(&mut current));
            used = base;
        }
        current.push(name.clone());
        used += units;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(count: usize) -> Vec<String> {
        let mut names: Vec<_> = (0..count)
            .map(|index| format!("module{}::tests::case_{index:05}", index % 7))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn libtest_lists_parse_only_terse_test_lines() {
        assert_eq!(
            parse_libtest_list("b::c: test\na: test\r\n").unwrap(),
            ["a", "b::c"]
        );
        assert!(parse_libtest_list("").unwrap().is_empty());
        for (text, reason) in [
            ("a: bench\n", "benchmark"),
            ("a: test\na: test\n", "more than once"),
            ("a\n", "unexpected libtest list line 1"),
            ("a: test\n\n", "unexpected libtest list line 2"),
            ("3 tests, 0 benchmarks\n", "unexpected"),
            (": test\n", "empty or padded"),
            (" a: test\n", "empty or padded"),
            ("a\u{7}b: test\n", "control character"),
        ] {
            let error = parse_libtest_list(text).unwrap_err().to_string();
            assert!(error.contains(reason), "{text:?}: {error}");
        }
    }

    #[test]
    fn list_digests_are_canonical() {
        let list = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(list_sha256(&list), crate::archive::digest(b"a\nb\n"));
        assert_eq!(list_sha256(&[]), crate::archive::digest(b""));
        assert_ne!(list_sha256(&list), list_sha256(&["ab".to_owned()]));
    }

    #[test]
    fn assignment_is_deterministic_complete_and_disjoint() {
        let listed = names(500);
        // A known value pins the function across releases and hosts.
        assert_eq!(assign("kuru-core/lib/kuru_core", "tests::a", 8), 6);
        for count in 1..=16 {
            let mut seen = BTreeSet::new();
            let mut sizes = Vec::new();
            for index in 1..=count {
                let scheme = PartitionScheme::new(index, count).unwrap();
                let placement = scheme.placement("x86_64-unknown-linux-gnu").unwrap();
                let assigned = scheme.assigned(&placement, "pkg/lib/pkg", &listed).unwrap();
                assert_eq!(
                    assigned,
                    scheme.assigned(&placement, "pkg/lib/pkg", &listed).unwrap()
                );
                sizes.push(assigned.len());
                for name in assigned {
                    assert!(seen.insert(name), "count {count} assigns twice");
                }
            }
            assert_eq!(seen.into_iter().collect::<Vec<_>>(), listed, "{count}");
            if count > 1 {
                // Balanced by count within a generous bound.
                let mean = 500 / count as usize;
                assert!(sizes.iter().all(|size| *size < mean * 2 + 10), "{sizes:?}");
            }
        }
        // The key, not the executable path, selects the partition.
        let spread: BTreeSet<_> = (0..64)
            .map(|index| assign(&format!("pkg{index}/lib/pkg"), "tests::a", 8))
            .collect();
        assert!(spread.len() > 1);
    }

    #[test]
    fn partition_schemes_validate_their_bounds() {
        assert!(PartitionScheme::new(1, 1).is_ok());
        assert!(PartitionScheme::new(64, 64).is_ok());
        for (index, count) in [(0, 1), (2, 1), (1, 0), (1, 65)] {
            assert!(
                PartitionScheme::new(index, count).is_err(),
                "{index}/{count}"
            );
        }
        let mut scheme = PartitionScheme::new(1, 2).unwrap();
        scheme.scheme = "other".to_owned();
        assert!(
            scheme
                .validate()
                .unwrap_err()
                .to_string()
                .contains("scheme")
        );
        // A plan placed with another table is refused, naming both digests.
        let mut scheme = PartitionScheme::new(1, 2).unwrap();
        scheme.timings_sha256 = "0".repeat(64);
        let error = scheme
            .placement("x86_64-unknown-linux-gnu")
            .unwrap_err()
            .to_string();
        assert!(error.contains(&"0".repeat(64)), "{error}");
        assert!(
            error.contains(timing::embedded_sha256().unwrap()),
            "{error}"
        );
        // A placement for another count cannot assign this partition.
        let scheme = PartitionScheme::new(1, 2).unwrap();
        let other = PartitionScheme::new(1, 3).unwrap();
        let placement = other.placement("x86_64-unknown-linux-gnu").unwrap();
        assert!(scheme.assigned(&placement, "pkg/lib/pkg", &[]).is_err());
    }

    #[test]
    fn artifact_keys_omit_paths_and_hashes() {
        let artifact = Artifact {
            package: "kuru-memory".to_owned(),
            package_root: "packages/kuru-memory".to_owned(),
            target_name: "kuru_memory".to_owned(),
            target_kind: vec!["lib".to_owned()],
            crate_types: vec!["lib".to_owned()],
            source: "packages/kuru-memory/src/lib.rs".to_owned(),
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
            filenames: vec!["debug/deps/kuru_memory-0123.exe".to_owned()],
            executable: Some("debug/deps/kuru_memory-0123.exe".to_owned()),
        };
        assert_eq!(artifact_key(&artifact), "kuru-memory/lib/kuru_memory");
    }

    #[test]
    fn windows_units_follow_command_line_quoting() {
        assert_eq!(argument_units("abc"), 5);
        assert_eq!(argument_units(""), 2);
        assert_eq!(argument_units("a b"), 5);
        assert_eq!(argument_units("a\"b"), 6);
        assert_eq!(argument_units("a\\\"b"), 8);
        assert_eq!(argument_units("a\\b"), 5);
        assert_eq!(argument_units("a b\\"), 7);
        assert_eq!(argument_units("C:\\x"), 6);
        assert_eq!(argument_units("é𝄞"), 5);
        // "C:\x" "--exact" "a"\0
        assert_eq!(windows_command_line_units("C:\\x", &["--exact", "a"]), 21);
    }

    #[test]
    fn many_short_names_stay_under_the_windows_limit_as_the_launcher_quotes_them() {
        let program =
            "D:\\a\\_temp\\kuru-coverage-target\\debug\\deps\\kuru_runtime-0123456789abcdef.exe";
        let listed: Vec<String> = (0..4000).map(|index| format!("t::c{index:05}")).collect();
        let chunks = chunk(program, &["--exact"], &listed, COMMAND_LINE_BUDGET).unwrap();
        assert!(chunks.len() > 1);
        assert!(chunks.iter().any(|chunk| chunk.len() > 1500));
        for chunk in &chunks {
            // Independent count: `"program" "--exact" "name"...` plus the
            // terminator, every argument quoted, all ASCII without quotes.
            let units = program.len()
                + 2
                + (1 + "--exact".len() + 2)
                + chunk.iter().map(|name| 1 + name.len() + 2).sum::<usize>()
                + 1;
            assert!(units <= COMMAND_LINE_BUDGET, "{units}");
            assert!(units < 32_767, "{units}");
        }
        assert_eq!(chunks.concat(), listed);
    }

    #[test]
    fn chunks_stay_under_budget_and_preserve_every_name_in_order() {
        let program =
            "D:\\a\\_temp\\kuru-coverage-target\\debug\\deps\\kuru_runtime-0123456789abcdef.exe";
        let listed = names(3000);
        let chunks = chunk(program, &["--exact"], &listed, COMMAND_LINE_BUDGET).unwrap();
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(!chunk.is_empty());
            let mut args = vec!["--exact"];
            args.extend(chunk.iter().map(String::as_str));
            assert!(windows_command_line_units(program, &args) <= COMMAND_LINE_BUDGET);
            assert!(windows_command_line_units(program, &args) < 32_767);
        }
        assert_eq!(chunks.concat(), listed);
        // A tight budget still makes progress one name at a time.
        let tight = windows_command_line_units(program, &["--exact", &listed[0]]);
        let singles = chunk(program, &["--exact"], &listed[..3], tight).unwrap();
        assert_eq!(singles.len(), 3);
        // Empty selections make no chunk; a name that cannot fit alone fails.
        assert!(chunk(program, &["--exact"], &[], 100).unwrap().is_empty());
        let error = chunk(program, &["--exact"], &["x".repeat(200)], 150)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not fit"), "{error}");
    }
}
