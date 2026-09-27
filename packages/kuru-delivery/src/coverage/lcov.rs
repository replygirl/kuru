//! Strict LCOV parsing, source normalization, union and the line gate.
//!
//! Each instrumented partition exports LCOV against the binaries that produced
//! its profiles and normalizes every source to a root-relative forward-slash
//! path. The merge requires every partition to describe identical files,
//! lines and functions (they were built from one agreed inventory), sums
//! their hit counts and computes the line total once for the OS.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Largest LCOV file a partition may export or the merge may read.
pub const LCOV_LIMIT: u64 = 256 * 1024 * 1024;
/// The per-OS line gate, in percent.
pub const LINE_GATE_PERCENT: u64 = 90;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileCoverage {
    /// Function name to (first line, hits).
    functions: BTreeMap<String, (u64, u64)>,
    /// Line to hits.
    lines: BTreeMap<u64, u64>,
}

/// Parsed coverage keyed by source path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Lcov {
    files: BTreeMap<String, FileCoverage>,
}

/// Line totals of one coverage set.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Totals {
    pub files: usize,
    pub lines_found: u64,
    pub lines_hit: u64,
}

fn number(text: &str, what: &str) -> Result<u64> {
    ensure!(
        !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()),
        "LCOV {what} {text:?} is not a decimal number"
    );
    text.parse()
        .with_context(|| format!("LCOV {what} {text:?} is out of range"))
}

impl Lcov {
    /// Parse the records llvm-cov writes. Branch data is refused because branch
    /// coverage is not enabled; summary records must equal the parsed data.
    pub fn parse(text: &str) -> Result<Self> {
        /// The open record: source, coverage and its summary lines.
        type Open<'a> = (String, FileCoverage, Vec<(&'a str, u64)>);
        let mut files = BTreeMap::new();
        let mut current: Option<Open<'_>> = None;
        for (index, line) in text.lines().enumerate() {
            let number_of = index + 1;
            let line = line.strip_suffix('\r').unwrap_or(line);
            let context = || format!("LCOV line {number_of}: {line:?}");
            if line.is_empty() || line.starts_with("TN:") {
                continue;
            }
            if let Some(path) = line.strip_prefix("SF:") {
                ensure!(current.is_none(), "{}: unterminated record", context());
                ensure!(!path.is_empty(), "{}: empty source", context());
                current = Some((path.to_owned(), FileCoverage::default(), Vec::new()));
                continue;
            }
            let Some((path, file, summaries)) = current.as_mut() else {
                bail!("{}: record outside a source file", context());
            };
            if line == "end_of_record" {
                let (path, file, summaries) = current.take().expect("open record");
                file.check_summaries(&summaries)
                    .with_context(|| format!("LCOV source {path}"))?;
                ensure!(
                    files.insert(path.clone(), file).is_none(),
                    "LCOV repeats source {path}"
                );
                continue;
            }
            let (tag, value) = line.split_once(':').with_context(context)?;
            match tag {
                "FN" => {
                    // `FN:<line>,<name>` or `FN:<line>,<end>,<name>`.
                    let (start, rest) = value.split_once(',').with_context(context)?;
                    let start = number(start, "function line")?;
                    let name = match rest.split_once(',') {
                        Some((end, name)) if number(end, "end line").is_ok() => name,
                        _ => rest,
                    };
                    ensure!(!name.is_empty(), "{}: empty function name", context());
                    ensure!(
                        file.functions.insert(name.to_owned(), (start, 0)).is_none(),
                        "{}: function repeated in {path}",
                        context()
                    );
                }
                "FNDA" => {
                    let (hits, name) = value.split_once(',').with_context(context)?;
                    let hits = number(hits, "function hits")?;
                    let entry = file
                        .functions
                        .get_mut(name)
                        .with_context(|| format!("{}: FNDA before its FN", context()))?;
                    entry.1 = entry
                        .1
                        .checked_add(hits)
                        .context("function hits overflow")?;
                }
                "DA" => {
                    let mut fields = value.split(',');
                    let (Some(line_number), Some(hits), None) =
                        (fields.next(), fields.next(), fields.next())
                    else {
                        bail!("{}: expected DA:<line>,<hits>", context());
                    };
                    ensure!(
                        file.lines
                            .insert(number(line_number, "line")?, number(hits, "line hits")?)
                            .is_none(),
                        "{}: line repeated in {path}",
                        context()
                    );
                }
                "FNF" | "FNH" | "LF" | "LH" => summaries.push((tag, number(value, tag)?)),
                "BRF" | "BRH" => ensure!(
                    number(value, tag)? == 0,
                    "{}: branch coverage is not enabled",
                    context()
                ),
                _ => bail!("{}: unsupported LCOV record", context()),
            }
        }
        ensure!(current.is_none(), "LCOV ends inside a source record");
        Ok(Self { files })
    }

    /// Rewrite every source to a forward-slash path relative to `root`,
    /// refusing a source outside it. Windows drive letters compare without case.
    pub fn normalize(self, root: &str) -> Result<Self> {
        let root = comparable(root);
        let root = root.trim_end_matches('/');
        ensure!(!root.is_empty(), "coverage root is empty");
        let mut files = BTreeMap::new();
        for (path, file) in self.files {
            let comparable_path = comparable(&path);
            let relative = comparable_path
                .strip_prefix(root)
                .and_then(|rest| rest.strip_prefix('/'))
                .with_context(|| format!("LCOV source {path} is outside {root}"))?;
            ensure!(
                relative
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != ".."),
                "LCOV source {path} is not a normal path below the root"
            );
            ensure!(
                files.insert(relative.to_owned(), file).is_none(),
                "LCOV source {path} normalizes onto another source"
            );
        }
        Ok(Self { files })
    }

    /// Require every source to already be a normalized relative path.
    pub fn require_relative(&self) -> Result<()> {
        for path in self.files.keys() {
            ensure!(
                !path.starts_with('/')
                    && !path.contains('\\')
                    && !path.contains(':')
                    && path
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != ".."),
                "LCOV source {path} is not a normalized relative path"
            );
        }
        Ok(())
    }

    pub fn totals(&self) -> Totals {
        Totals {
            files: self.files.len(),
            lines_found: self
                .files
                .values()
                .map(|file| file.lines.len() as u64)
                .sum(),
            lines_hit: self
                .files
                .values()
                .map(|file| file.lines.values().filter(|hits| **hits > 0).count() as u64)
                .sum(),
        }
    }

    /// Add another partition's counts. Its sources, lines and functions must
    /// be exactly this set's.
    pub fn absorb(&mut self, other: &Self, label: &str) -> Result<()> {
        ensure!(
            self.files.keys().eq(other.files.keys()),
            "{label} covers different source files"
        );
        for (path, file) in &mut self.files {
            let theirs = &other.files[path];
            ensure!(
                file.lines.keys().eq(theirs.lines.keys()),
                "{label} instruments different lines in {path}"
            );
            ensure!(
                file.functions.len() == theirs.functions.len()
                    && file.functions.iter().all(|(name, (line, _))| theirs
                        .functions
                        .get(name)
                        .is_some_and(|(other, _)| other == line)),
                "{label} instruments different functions in {path}"
            );
            for (line, hits) in &mut file.lines {
                *hits = hits
                    .checked_add(theirs.lines[line])
                    .context("line hits overflow")?;
            }
            for (name, (_, hits)) in &mut file.functions {
                *hits = hits
                    .checked_add(theirs.functions[name].1)
                    .context("function hits overflow")?;
            }
        }
        Ok(())
    }

    /// Canonical LCOV with recomputed summaries.
    pub fn render(&self) -> String {
        use std::fmt::Write as _;

        let mut text = String::new();
        for (path, file) in &self.files {
            let _ = writeln!(text, "SF:{path}");
            let mut functions: Vec<_> = file.functions.iter().collect();
            functions.sort_by(|left, right| (left.1.0, left.0).cmp(&(right.1.0, right.0)));
            for (name, (line, _)) in &functions {
                let _ = writeln!(text, "FN:{line},{name}");
            }
            for (name, (_, hits)) in &functions {
                let _ = writeln!(text, "FNDA:{hits},{name}");
            }
            let _ = writeln!(text, "FNF:{}", file.functions.len());
            let _ = writeln!(
                text,
                "FNH:{}",
                file.functions
                    .values()
                    .filter(|(_, hits)| *hits > 0)
                    .count()
            );
            for (line, hits) in &file.lines {
                let _ = writeln!(text, "DA:{line},{hits}");
            }
            let _ = writeln!(text, "LF:{}", file.lines.len());
            let _ = writeln!(
                text,
                "LH:{}",
                file.lines.values().filter(|hits| **hits > 0).count()
            );
            text.push_str("end_of_record\n");
        }
        text
    }
}

impl FileCoverage {
    /// llvm-cov writes its summaries from its own report model: `FNF`/`FNH`
    /// count deduplicated functions while `FN` lists every instantiation, and
    /// `LF`/`LH` sum each function group's lines, so a line shared by a
    /// closure and its parent counts twice. Neither can be derived from the
    /// records or unioned across partitions, so they are accepted as given,
    /// only once each, and the rendered summaries are recomputed from the
    /// records: the gate counts unique instrumented lines (`DA`).
    fn check_summaries(&self, summaries: &[(&str, u64)]) -> Result<()> {
        let mut tags: Vec<_> = summaries.iter().map(|(tag, _)| *tag).collect();
        tags.sort_unstable();
        ensure!(
            tags.windows(2).all(|pair| pair[0] != pair[1]),
            "summary record repeated"
        );
        Ok(())
    }
}

/// A path in comparable form: forward slashes, no Windows verbatim prefix and
/// a lowercase drive letter.
fn comparable(path: &str) -> String {
    let mut path = path.replace('\\', "/");
    if let Some(rest) = path.strip_prefix("//?/") {
        path = rest.to_owned();
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        path.replace_range(..1, &path[..1].to_ascii_lowercase());
    }
    path
}

/// Whether `lines_hit / lines_found` reaches the gate, over unique instrumented
/// lines (`DA` records). No lines never passes.
pub fn passes_gate(totals: &Totals) -> bool {
    totals.lines_found > 0
        && u128::from(totals.lines_hit) * 100
            >= u128::from(totals.lines_found) * u128::from(LINE_GATE_PERCENT)
}

/// Percentage with two decimals, for logs only.
pub fn percent(totals: &Totals) -> String {
    if totals.lines_found == 0 {
        return "0.00".to_owned();
    }
    let hundredths = u128::from(totals.lines_hit) * 10_000 / u128::from(totals.lines_found);
    format!("{}.{:02}", hundredths / 100, hundredths % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "SF:/work/kuru/apps/a.rs\nFN:3,_RNvf\nFN:9,_RNvg\nFNDA:2,_RNvf\nFNDA:0,_RNvg\nFNF:2\nFNH:1\nDA:3,2\nDA:4,0\nDA:9,0\nBRF:0\nBRH:0\nLF:3\nLH:1\nend_of_record\nSF:/work/kuru/packages/b.rs\nFN:1,2,_RNvh\nFNDA:1,_RNvh\nDA:1,1\nend_of_record\n";

    #[test]
    fn llvm_cov_records_parse_and_render_canonically() {
        let lcov = Lcov::parse(RAW).unwrap().normalize("/work/kuru/").unwrap();
        assert_eq!(
            lcov.totals(),
            Totals {
                files: 2,
                lines_found: 4,
                lines_hit: 2
            }
        );
        let rendered = lcov.render();
        assert!(rendered.starts_with("SF:apps/a.rs\nFN:3,_RNvf\nFN:9,_RNvg\nFNDA:2,_RNvf\n"));
        assert!(rendered.contains("SF:packages/b.rs\nFN:1,_RNvh\nFNDA:1,_RNvh\nFNF:1\nFNH:1\nDA:1,1\nLF:1\nLH:1\nend_of_record\n"));
        // The canonical form round-trips.
        let reparsed = Lcov::parse(&rendered).unwrap();
        reparsed.require_relative().unwrap();
        assert_eq!(reparsed, lcov);
        assert_eq!(reparsed.render(), rendered);
    }

    #[test]
    fn summaries_follow_llvm_cov_and_are_recomputed_from_records() {
        // cargo-llvm-cov 0.9.1 counts deduplicated functions in FNF/FNH but
        // lists every instantiation as FN, and sums LF/LH per function group
        // (captured: FN 25 vs FNF 12, DA 95 vs LF 103 in one real file).
        let text = "SF:/r/a.rs\nFN:3,_RNvf1\nFN:3,_RNvf2\nFNDA:1,_RNvf1\nFNDA:0,_RNvf2\nFNF:1\nFNH:1\nDA:3,1\nDA:4,0\nBRF:0\nBRH:0\nLF:3\nLH:2\nend_of_record\n";
        let lcov = Lcov::parse(text).unwrap().normalize("/r").unwrap();
        let rendered = lcov.render();
        assert!(rendered.contains("FNF:2\nFNH:1\n"), "{rendered}");
        assert!(rendered.contains("LF:2\nLH:1\n"), "{rendered}");
        assert_eq!(lcov.totals().lines_found, 2);
    }

    #[test]
    fn malformed_and_branch_records_are_refused() {
        for (text, reason) in [
            ("DA:1,1\n", "outside a source"),
            ("SF:a\nSF:b\n", "unterminated"),
            ("SF:a\nDA:1,1\n", "ends inside"),
            ("SF:a\nDA:1\nend_of_record\n", "expected DA"),
            ("SF:a\nDA:1,1\nDA:1,2\nend_of_record\n", "line repeated"),
            ("SF:a\nDA:x,1\nend_of_record\n", "not a decimal"),
            ("SF:a\nDA:1,-1\nend_of_record\n", "not a decimal"),
            ("SF:a\nBRDA:1,0,0,1\nend_of_record\n", "unsupported"),
            ("SF:a\nBRF:2\nend_of_record\n", "branch coverage"),
            ("SF:a\nFNDA:1,f\nend_of_record\n", "FNDA before"),
            ("SF:a\nFN:1,f\nFN:2,f\nend_of_record\n", "function repeated"),
            ("SF:a\nFN:1,\nend_of_record\n", "empty function"),
            ("SF:a\nDA:1,1\nLF:1\nLF:1\nend_of_record\n", "repeated"),
            (
                "SF:a\nend_of_record\nSF:a\nend_of_record\n",
                "repeats source",
            ),
            ("SF:\n", "empty source"),
            ("SF:a\nXYZ:1\nend_of_record\n", "unsupported"),
            ("SF:a\nnonsense\nend_of_record\n", "LCOV line 2"),
            (
                "SF:a\nDA:99999999999999999999999,1\nend_of_record\n",
                "out of range",
            ),
        ] {
            let error = format!("{:#}", Lcov::parse(text).unwrap_err());
            assert!(error.contains(reason), "{text:?}: {error}");
        }
    }

    #[test]
    fn sources_normalize_below_the_root_with_folded_drive_case() {
        let windows = "SF:D:\\a\\kuru\\kuru\\apps\\x.rs\nDA:1,1\nend_of_record\n";
        let lcov = Lcov::parse(windows)
            .unwrap()
            .normalize("d:\\a\\kuru\\kuru")
            .unwrap();
        assert_eq!(lcov.files.keys().collect::<Vec<_>>(), ["apps/x.rs"]);
        let verbatim = "SF:\\\\?\\D:\\a\\kuru\\kuru\\apps\\x.rs\nDA:1,1\nend_of_record\n";
        let lcov = Lcov::parse(verbatim)
            .unwrap()
            .normalize("D:/a/kuru/kuru")
            .unwrap();
        assert_eq!(lcov.files.keys().collect::<Vec<_>>(), ["apps/x.rs"]);
        for (root, reason) in [
            ("/elsewhere", "outside"),
            ("D:\\a\\kuru\\kuru\\apps\\x.rs", "outside"),
            ("", "empty"),
        ] {
            let error = Lcov::parse(windows)
                .unwrap()
                .normalize(root)
                .unwrap_err()
                .to_string();
            assert!(error.contains(reason), "{root:?}: {error}");
        }
        let dotted = "SF:/r/a/../b.rs\nDA:1,1\nend_of_record\n";
        assert!(Lcov::parse(dotted).unwrap().normalize("/r").is_err());
        let colliding = "SF:/r/a.rs\nDA:1,1\nend_of_record\nSF:/r\\a.rs\nDA:1,1\nend_of_record\n";
        assert!(
            Lcov::parse(colliding)
                .unwrap()
                .normalize("/r")
                .unwrap_err()
                .to_string()
                .contains("onto another")
        );
        for path in ["/abs.rs", "a\\b.rs", "c:/x.rs", "a/../b.rs", "a//b.rs"] {
            let text = format!("SF:{path}\nDA:1,1\nend_of_record\n");
            assert!(
                Lcov::parse(&text).unwrap().require_relative().is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn partitions_union_counts_only_over_identical_structure() {
        let base = Lcov::parse(RAW).unwrap().normalize("/work/kuru").unwrap();
        let other_text = RAW
            .replace("DA:4,0", "DA:4,5")
            .replace("FNDA:0,_RNvg", "FNDA:1,_RNvg")
            .replace("FNH:1", "FNH:2")
            .replace("LH:1", "LH:2");
        let other = Lcov::parse(&other_text)
            .unwrap()
            .normalize("/work/kuru")
            .unwrap();
        let mut merged = base.clone();
        merged.absorb(&other, "partition 2").unwrap();
        assert_eq!(merged.totals().lines_hit, 3);
        assert!(merged.render().contains("DA:4,5\n"));
        assert!(merged.render().contains("FNDA:1,_RNvg\nFNF:2\nFNH:2\n"));
        assert!(merged.render().contains("DA:3,4\n"));

        for (text, reason) in [
            (
                RAW.replace("SF:/work/kuru/packages/b.rs", "SF:/work/kuru/packages/c.rs"),
                "different source files",
            ),
            (RAW.replace("DA:9,0\n", "DA:10,0\n"), "different lines"),
            (
                RAW.replace("FN:9,_RNvg", "FN:8,_RNvg"),
                "different functions",
            ),
            (
                RAW.replace("FN:9,_RNvg", "FN:9,_RNvz")
                    .replace("FNDA:0,_RNvg", "FNDA:0,_RNvz"),
                "different functions",
            ),
        ] {
            let other = Lcov::parse(&text).unwrap().normalize("/work/kuru").unwrap();
            let error = base
                .clone()
                .absorb(&other, "partition 3")
                .unwrap_err()
                .to_string();
            assert!(error.contains(reason), "{error}");
            assert!(error.contains("partition 3"), "{error}");
        }
    }

    #[test]
    fn the_gate_passes_at_exactly_ninety_percent_and_fails_below() {
        let totals = |found, hit| Totals {
            files: 1,
            lines_found: found,
            lines_hit: hit,
        };
        assert!(passes_gate(&totals(1000, 900)));
        assert!(!passes_gate(&totals(1000, 899)));
        assert!(passes_gate(&totals(10, 10)));
        assert!(!passes_gate(&totals(0, 0)));
        assert!(!passes_gate(&totals(100_001, 90_000)));
        assert_eq!(percent(&totals(1000, 900)), "90.00");
        assert_eq!(percent(&totals(3, 2)), "66.66");
        assert_eq!(percent(&totals(0, 0)), "0.00");
    }
}
