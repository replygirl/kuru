//! cargo-llvm-cov's line metric, reproduced exactly across partitions.
//!
//! `--fail-under-lines` reads `totals.lines` of `llvm-cov export
//! -format=text` (cargo-llvm-cov 0.9.1, `report.rs`). llvm-cov (LLVM 22.1.8,
//! the rustc 1.98.1 llvm-tools) computes that total per file as a sum over
//! instantiation groups: the functions whose main view is the file, grouped by
//! the start of their first region in that file
//! (`FunctionInstantiationSetCollector`). Each group counts the most mapped
//! and the most covered lines of any of its instantiations
//! (`LineCoverageInfo::merge`), and each instantiation's lines come from the
//! segments `SegmentBuilder` builds over its main-file regions and the
//! `LineCoverageStats` of each line.
//!
//! Each instrumented partition ports that derivation over its own export,
//! records every instantiation's mapped and covered lines, and must reproduce
//! its own `--summary-only` figures exactly per file and in total. The merge
//! unions each instantiation's covered lines across partitions and takes each
//! group's maximum. That equals the summary of the summed profile because the
//! partitions' mappings are identical and their region counts non-negative: a
//! region's summed count is nonzero exactly when some partition's is, and a
//! line's count is the maximum of a structurally chosen set of region counts.
//! Non-negativity is enforced: a region carrying the clamped count of a
//! negative counter expression fails the partition's line export.
//!
//! The argument also needs every partition region to be a counted code
//! region of nonzero length, which is all rustc emits. llvm-cov suppresses
//! segments for skipped and zero-length regions depending on the counts
//! around them, so with those present a line's figure would no longer be a
//! count-independent structural choice. The partition export
//! ([`LlvmExport::partition_line_export`]) refuses them, naming the function;
//! the general [`LlvmExport::line_export`] derives every region kind, which
//! the clang fixtures exercise.

use super::lcov::relative_source;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

/// Largest raw `llvm-cov export` JSON a partition may read.
pub const EXPORT_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
/// Largest line export a partition may write or the merge may read.
pub const LINES_LIMIT: u64 = 256 * 1024 * 1024;
/// The per-OS line gate: the percent `mise run coverage` passes to
/// cargo-llvm-cov's `--fail-under-lines`, over the same metric.
pub const GATE_PERCENT: u64 = 95;
/// Schema of the per-partition line export.
pub const LINES_SCHEMA: u32 = 1;
/// The `llvm-cov export` document type and version this port reproduces.
const EXPORT_TYPE: &str = "llvm.coverage.json.export";
const EXPORT_VERSION: &str = "3.1.0";
/// Mismatching files named in one diagnostic.
const MISMATCH_REPORT_LIMIT: usize = 10;

/// The parts of an `llvm-cov export -format=text` document the port reads,
/// as cargo-llvm-cov writes it with `--json` (full or `--summary-only`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlvmExport {
    data: Vec<ExportData>,
    #[serde(rename = "type")]
    kind: String,
    version: String,
    /// cargo-llvm-cov's own identification, which it injects.
    #[serde(default)]
    #[allow(dead_code)]
    cargo_llvm_cov: Option<IgnoredAny>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportData {
    files: Vec<ExportFile>,
    /// Absent from a summary-only export.
    #[serde(default)]
    functions: Option<Vec<ExportFunction>>,
    totals: ExportSummary,
}

#[derive(Debug, Deserialize)]
struct ExportFile {
    filename: String,
    summary: ExportSummary,
}

#[derive(Debug, Deserialize)]
struct ExportSummary {
    lines: ExportLines,
}

#[derive(Debug, Deserialize)]
struct ExportLines {
    count: u64,
    covered: u64,
}

#[derive(Debug, Deserialize)]
struct ExportFunction {
    name: String,
    filenames: Vec<String>,
    /// `[line start, column start, line end, column end, count, file id,
    /// expanded file id, kind]`, in the function's record order.
    regions: Vec<[u64; 8]>,
}

impl LlvmExport {
    /// Read a bounded export file.
    pub fn read(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let length = file.metadata()?.len();
        ensure!(
            length <= EXPORT_LIMIT,
            "{} is {length} bytes, above the {EXPORT_LIMIT}-byte limit",
            path.display()
        );
        let export: Self = serde_json::from_reader(BufReader::new(file.take(EXPORT_LIMIT)))
            .with_context(|| format!("parse llvm-cov export {}", path.display()))?;
        export.check_version()?;
        Ok(export)
    }

    #[cfg(test)]
    pub fn parse(text: &str) -> Result<Self> {
        let export: Self = serde_json::from_str(text)?;
        export.check_version()?;
        Ok(export)
    }

    fn check_version(&self) -> Result<()> {
        ensure!(
            self.kind == EXPORT_TYPE && self.version == EXPORT_VERSION,
            "llvm-cov export is {} {}, but this port reproduces {EXPORT_TYPE} {EXPORT_VERSION}",
            self.kind,
            self.version
        );
        ensure!(
            self.data.len() == 1,
            "llvm-cov export holds {} coverage mappings, expected one",
            self.data.len()
        );
        Ok(())
    }

    fn data(&self) -> &ExportData {
        &self.data[0]
    }

    /// The line figures llvm-cov itself reports, per root-relative file and
    /// in total.
    pub fn summary_figures(&self, root: &str) -> Result<Figures> {
        let data = self.data();
        let mut files = BTreeMap::new();
        for file in &data.files {
            let relative = relative_source(&file.filename, root)?;
            let lines = Lines {
                count: file.summary.lines.count,
                covered: file.summary.lines.covered,
            };
            ensure!(
                lines.covered <= lines.count,
                "llvm-cov reports more covered than mapped lines in {relative}"
            );
            ensure!(
                files.insert(relative, lines).is_none(),
                "llvm-cov export source {} normalizes onto another source",
                file.filename
            );
        }
        Ok(Figures {
            files,
            total: Lines {
                count: data.totals.lines.count,
                covered: data.totals.lines.covered,
            },
        })
    }

    /// Every instantiation's mapped and covered lines, as llvm-cov derives
    /// them, for the functions whose main view is an exported source file.
    /// Accepts every region kind.
    pub fn line_export(&self, root: &str) -> Result<LineExport> {
        self.derive_lines(root, Accept::AnyRegion)
    }

    /// The line export a coverage partition writes: [`Self::line_export`],
    /// refusing any main-file region that is not a counted code region of
    /// nonzero length, since only over those does the partitions' union
    /// equal the summed profile's figure exactly.
    pub fn partition_line_export(&self, root: &str) -> Result<LineExport> {
        self.derive_lines(root, Accept::CountedCode)
    }

    fn derive_lines(&self, root: &str, accept: Accept) -> Result<LineExport> {
        let data = self.data();
        let functions = data
            .functions
            .as_ref()
            .context("llvm-cov export has no functions; it is summary-only")?;
        let mut sources = BTreeMap::new();
        for file in &data.files {
            let relative = relative_source(&file.filename, root)?;
            ensure!(
                sources.insert(file.filename.as_str(), relative).is_none(),
                "llvm-cov export repeats source {}",
                file.filename
            );
        }
        let mut instantiations: BTreeMap<(String, String), Instantiation> = BTreeMap::new();
        for function in functions {
            let Some(main) = main_file(function)? else {
                continue;
            };
            let Some(file) = sources.get(function.filenames[main].as_str()) else {
                // A source llvm-cov was told to ignore.
                continue;
            };
            let regions = function
                .regions
                .iter()
                .filter(|region| region[5] == main as u64)
                .map(Region::parse)
                .collect::<Result<Vec<_>>>()
                .with_context(|| format!("function {} in {file}", function.name))?;
            if accept == Accept::CountedCode
                && let Some(region) = regions
                    .iter()
                    .find(|region| region.kind != Kind::Code || region.start == region.end)
            {
                bail!(
                    "function {} in {file}: region {region:?} is not a counted code region of \
                     nonzero length; llvm-cov's segment suppression for skipped and zero-length \
                     regions depends on counts, so a partition's line export refuses them",
                    function.name
                );
            }
            let first = regions
                .first()
                .with_context(|| format!("function {} has no region in {file}", function.name))?;
            let (line, column) = first.start;
            let (mapped, covered) = line_statistics(&build_segments(regions));
            let key = (file.clone(), function.name.clone());
            ensure!(
                !instantiations.contains_key(&key),
                "llvm-cov export repeats function {} in {file}",
                function.name
            );
            instantiations.insert(
                key,
                Instantiation {
                    file: file.clone(),
                    name: function.name.clone(),
                    line,
                    column,
                    mapped: LineSet::from_sorted(mapped),
                    covered: LineSet::from_sorted(covered),
                },
            );
        }
        let files: BTreeSet<String> = sources.values().cloned().collect();
        ensure!(
            files.len() == sources.len(),
            "llvm-cov export sources normalize onto one another"
        );
        let export = LineExport {
            schema: LINES_SCHEMA,
            files: files.into_iter().collect(),
            instantiations: instantiations.into_values().collect(),
        };
        export.validate()?;
        Ok(export)
    }
}

/// Which main-file regions a line export derives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Accept {
    /// Every region kind llvm-cov exports.
    AnyRegion,
    /// Only code regions of nonzero length: a partition's line export.
    CountedCode,
}

/// The first file id that no expansion region expands into
/// (`findMainViewFileID`).
fn main_file(function: &ExportFunction) -> Result<Option<usize>> {
    let mut expanded = vec![false; function.filenames.len()];
    for region in &function.regions {
        if region[7] == Kind::Expansion as u64 {
            let id = usize::try_from(region[6])
                .ok()
                .filter(|id| *id < expanded.len());
            let id = id.with_context(|| {
                format!(
                    "function {} expands into unknown file {}",
                    function.name, region[6]
                )
            })?;
            expanded[id] = true;
        }
    }
    Ok(expanded.iter().position(|expanded| !expanded))
}

/// Region kinds llvm-cov exports in a function's `regions`. Branch and MC/DC
/// regions are exported separately.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    Code = 0,
    Expansion = 1,
    Skipped = 2,
    Gap = 3,
}

type Location = (u32, u32);

#[derive(Clone, Debug)]
struct Region {
    start: Location,
    end: Location,
    count: u64,
    kind: Kind,
}

/// `i64::MAX`, the value `llvm-cov export` gives a negative region count.
const NEGATIVE_COUNT: u64 = i64::MAX as u64;

fn position(value: u64, what: &str) -> Result<u32> {
    u32::try_from(value).with_context(|| format!("region {what} {value} is out of range"))
}

impl Region {
    fn parse(fields: &[u64; 8]) -> Result<Self> {
        let kind = match fields[7] {
            0 => Kind::Code,
            1 => Kind::Expansion,
            2 => Kind::Skipped,
            3 => Kind::Gap,
            other => bail!("unsupported region kind {other}"),
        };
        let region = Self {
            start: (position(fields[0], "line")?, position(fields[1], "column")?),
            end: (position(fields[2], "line")?, position(fields[3], "column")?),
            count: fields[4],
            kind,
        };
        ensure!(
            region.start <= region.end,
            "region ends before it starts: {region:?}"
        );
        // `llvm-cov export` clamps the huge unsigned value of a negative
        // counter expression (such as instrumented code still running on a
        // detached thread while the process writes its profile at exit, so the
        // written counters are mutually inconsistent) to i64::MAX; no real
        // count approaches it. A
        // negative count could cancel in the summed profile while this
        // partition reports the line covered, so the union would no longer
        // equal llvm-cov's merged figure. Refuse it and let the partition rerun.
        ensure!(
            region.count < NEGATIVE_COUNT,
            "region {region:?} has count {}, the clamped value of a negative counter expression; \
             the merged figure would not be exact, so rerun this partition",
            region.count
        );
        Ok(region)
    }
}

/// A `CoverageSegment`.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Segment {
    line: u32,
    column: u32,
    count: u64,
    has_count: bool,
    region_entry: bool,
    gap: bool,
}

/// `SegmentBuilder::buildSegments` over one function's main-file regions.
fn build_segments(mut regions: Vec<Region>) -> Vec<Segment> {
    // sortNestedRegions: by start, then the enclosing region first, then kind.
    regions.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| right.end.cmp(&left.end))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    // combineRegions: regions over one area become the first, which adds
    // the counts of the others of its kind.
    let mut combined: Vec<Region> = Vec::with_capacity(regions.len());
    for region in regions {
        match combined.last_mut() {
            Some(active) if active.start == region.start && active.end == region.end => {
                if active.kind == region.kind {
                    active.count = active.count.wrapping_add(region.count);
                }
            }
            _ => combined.push(region),
        }
    }
    let mut builder = SegmentBuilder {
        regions: &combined,
        segments: Vec::new(),
        active: Vec::new(),
    };
    builder.build();
    builder.segments
}

struct SegmentBuilder<'a> {
    regions: &'a [Region],
    segments: Vec<Segment>,
    /// Indices into `regions`.
    active: Vec<usize>,
}

impl SegmentBuilder<'_> {
    fn end(&self, index: usize) -> Location {
        self.regions[index].end
    }

    /// `startSegment`.
    fn start_segment(
        &mut self,
        index: usize,
        location: Location,
        region_entry: bool,
        emit_skipped: bool,
    ) {
        let region = &self.regions[index];
        let has_count = !emit_skipped && region.kind != Kind::Skipped;
        if !region_entry
            && !emit_skipped
            && self.segments.last().is_some_and(|last| {
                last.has_count == has_count && last.count == region.count && !last.region_entry
            })
        {
            return;
        }
        self.segments.push(Segment {
            line: location.0,
            column: location.1,
            count: if has_count { region.count } else { 0 },
            has_count,
            region_entry,
            gap: has_count && region.kind == Kind::Gap,
        });
    }

    /// `completeRegionsUntil`.
    fn complete_until(&mut self, location: Option<Location>, first_completed: usize) {
        let mut completed = self.active.split_off(first_completed);
        completed.sort_by_key(|index| self.regions[*index].end);
        self.active.extend(completed);
        for position in first_completed + 1..self.active.len() {
            let mut region = self.active[position];
            let segment_location = self.end(self.active[position - 1]);
            if location == Some(segment_location) {
                break;
            }
            if segment_location == self.end(region) {
                continue;
            }
            for later in position + 1..self.active.len() {
                if self.end(region) == self.end(self.active[later]) {
                    region = self.active[later];
                }
            }
            self.start_segment(region, segment_location, false, false);
        }
        let last = *self.active.last().expect("active regions");
        let last_end = self.end(last);
        if first_completed != 0 && location != Some(last_end) {
            self.start_segment(self.active[first_completed - 1], last_end, false, false);
        } else if first_completed == 0 && location != Some(last_end) {
            self.start_segment(last, last_end, false, true);
        }
        self.active.truncate(first_completed);
    }

    /// `buildSegmentsImpl`.
    fn build(&mut self) {
        let count = self.regions.len();
        for index in 0..count {
            let region = &self.regions[index];
            let start = region.start;
            // A stable partition: still-active regions first.
            let (open, done): (Vec<usize>, Vec<usize>) = self
                .active
                .iter()
                .partition(|active| !(self.end(**active) <= start));
            if !done.is_empty() {
                let first_completed = open.len();
                self.active = open;
                self.active.extend(done);
                self.complete_until(Some(start), first_completed);
            }
            let region = &self.regions[index];
            let gap = region.kind == Kind::Gap;
            if start == region.end {
                let skipped = index + 1 == count || region.kind == Kind::Skipped;
                let source = self.active.last().copied().unwrap_or(index);
                self.start_segment(source, start, !gap, skipped);
                if skipped && let Some(last) = self.active.last().copied() {
                    self.start_segment(last, start, false, false);
                }
                continue;
            }
            if index + 1 == count || start != self.regions[index + 1].start {
                self.start_segment(index, start, !gap, false);
            }
            self.active.push(index);
        }
        if !self.active.is_empty() {
            self.complete_until(None, 0);
        }
    }
}

/// `LineCoverageIterator` and `LineCoverageStats`: the mapped lines and the
/// mapped lines with a nonzero count, ascending.
fn line_statistics(segments: &[Segment]) -> (Vec<u32>, Vec<u32>) {
    let (mut mapped, mut covered) = (Vec::new(), Vec::new());
    let Some(first) = segments.first() else {
        return (mapped, covered);
    };
    let starts_region =
        |segment: &Segment| !segment.gap && segment.has_count && segment.region_entry;
    let mut line = first.line;
    let mut next = 0;
    let mut wrapped: Option<&Segment> = None;
    let mut on_line: Vec<&Segment> = Vec::new();
    while next < segments.len() {
        if let Some(last) = on_line.last() {
            wrapped = Some(*last);
        }
        on_line.clear();
        while next < segments.len() && segments[next].line == line {
            on_line.push(&segments[next]);
            next += 1;
        }
        let region_starts = on_line
            .iter()
            .filter(|segment| starts_region(segment))
            .count();
        let skipped_start = on_line
            .first()
            .is_some_and(|segment| !segment.has_count && segment.region_entry);
        let is_mapped = (!skipped_start
            && (wrapped.is_some_and(|segment| segment.has_count) || region_starts > 0))
            || on_line
                .iter()
                .any(|segment| segment.region_entry && segment.has_count);
        if is_mapped {
            let mut count = wrapped.map_or(0, |segment| segment.count);
            if region_starts > 0 {
                for segment in on_line.iter().filter(|segment| starts_region(segment)) {
                    count = count.max(segment.count);
                }
            }
            mapped.push(line);
            if count != 0 {
                covered.push(line);
            }
        }
        line += 1;
    }
    (mapped, covered)
}

/// Ascending line numbers, serialized as inclusive `[first, last]` runs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LineSet(Vec<u32>);

impl LineSet {
    pub(super) fn from_sorted(lines: Vec<u32>) -> Self {
        debug_assert!(lines.windows(2).all(|pair| pair[0] < pair[1]));
        Self(lines)
    }

    pub fn count(&self) -> u64 {
        self.0.len() as u64
    }

    fn is_subset(&self, other: &Self) -> bool {
        let other: BTreeSet<_> = other.0.iter().collect();
        self.0.iter().all(|line| other.contains(line))
    }

    fn union(&mut self, other: &Self) {
        let merged: BTreeSet<u32> = self.0.iter().chain(&other.0).copied().collect();
        self.0 = merged.into_iter().collect();
    }
}

impl Serialize for LineSet {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut runs: Vec<[u32; 2]> = Vec::new();
        for line in &self.0 {
            match runs.last_mut() {
                Some(run) if run[1].checked_add(1) == Some(*line) => run[1] = *line,
                _ => runs.push([*line, *line]),
            }
        }
        runs.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LineSet {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let runs = Vec::<[u32; 2]>::deserialize(deserializer)?;
        let mut lines = Vec::new();
        for run in &runs {
            // Canonical runs ascend, never touch and never span zero.
            if run[0] > run[1]
                || lines
                    .last()
                    .is_some_and(|last: &u32| u64::from(*last) + 1 >= u64::from(run[0]))
            {
                return Err(D::Error::custom(format!(
                    "line runs are not canonical at {run:?}"
                )));
            }
            if u64::from(run[1] - run[0]) + lines.len() as u64 > LINES_LIMIT {
                return Err(D::Error::custom("line runs exceed the export limit"));
            }
            lines.extend(run[0]..=run[1]);
        }
        Ok(Self(lines))
    }
}

/// One function instantiation's lines in its main file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Instantiation {
    pub file: String,
    /// The function record's name, unique within its file.
    pub name: String,
    /// Start of the function's first region in `file`: its group.
    pub line: u32,
    pub column: u32,
    pub mapped: LineSet,
    pub covered: LineSet,
}

/// One partition's line export, sorted by file and name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LineExport {
    pub schema: u32,
    /// Every root-relative source file llvm-cov reported, sorted.
    pub files: Vec<String>,
    pub instantiations: Vec<Instantiation>,
}

/// Mapped and covered lines.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lines {
    pub count: u64,
    pub covered: u64,
}

impl Lines {
    fn add(&mut self, other: Self) -> Result<()> {
        self.count = self
            .count
            .checked_add(other.count)
            .context("line count overflow")?;
        self.covered = self
            .covered
            .checked_add(other.covered)
            .context("line count overflow")?;
        Ok(())
    }
}

/// Per-file and total line figures.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Figures {
    pub files: BTreeMap<String, Lines>,
    pub total: Lines,
}

impl LineExport {
    /// Parse and validate a written export.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let export: Self = serde_json::from_slice(bytes).context("parse the line export")?;
        export.validate()?;
        Ok(export)
    }

    /// The canonical compact form.
    pub fn render(&self) -> Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == LINES_SCHEMA,
            "line export schema {} is not {LINES_SCHEMA}",
            self.schema
        );
        ensure!(
            self.files.windows(2).all(|pair| pair[0] < pair[1]),
            "line export files are not sorted and unique"
        );
        for file in &self.files {
            ensure!(
                !file.starts_with('/')
                    && !file.contains('\\')
                    && !file.contains(':')
                    && file
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != ".."),
                "line export source {file} is not a normalized relative path"
            );
        }
        ensure!(
            self.instantiations
                .windows(2)
                .all(|pair| (&pair[0].file, &pair[0].name) < (&pair[1].file, &pair[1].name)),
            "line export instantiations are not sorted and unique"
        );
        for instantiation in &self.instantiations {
            ensure!(
                self.files.binary_search(&instantiation.file).is_ok(),
                "line export instantiation {} names unlisted source {}",
                instantiation.name,
                instantiation.file
            );
            ensure!(
                !instantiation.name.is_empty(),
                "line export has an unnamed instantiation"
            );
            ensure!(
                instantiation.covered.is_subset(&instantiation.mapped),
                "line export instantiation {} covers unmapped lines",
                instantiation.name
            );
        }
        Ok(())
    }

    /// llvm-cov's figures: per file, the sum over instantiation groups of the
    /// most mapped and the most covered lines of any instantiation.
    pub fn figures(&self) -> Result<Figures> {
        let mut groups: BTreeMap<(&str, u32, u32), Lines> = BTreeMap::new();
        for instantiation in &self.instantiations {
            let group = groups
                .entry((
                    instantiation.file.as_str(),
                    instantiation.line,
                    instantiation.column,
                ))
                .or_default();
            group.count = group.count.max(instantiation.mapped.count());
            group.covered = group.covered.max(instantiation.covered.count());
        }
        let mut figures = Figures {
            files: self
                .files
                .iter()
                .map(|file| (file.clone(), Lines::default()))
                .collect(),
            total: Lines::default(),
        };
        for ((file, _, _), lines) in groups {
            figures
                .files
                .get_mut(file)
                .expect("validated file")
                .add(lines)?;
            figures.total.add(lines)?;
        }
        Ok(figures)
    }

    pub fn instantiation_count(&self) -> usize {
        self.instantiations.len()
    }
}

/// Require the port's figures to equal llvm-cov's own, per file and in total.
pub fn self_check(derived: &Figures, reported: &Figures) -> Result<()> {
    let mut mismatches = Vec::new();
    for (file, lines) in &reported.files {
        match derived.files.get(file) {
            Some(ours) if ours == lines => {}
            Some(ours) => mismatches.push(format!(
                "{file}: llvm-cov {}/{}, port {}/{}",
                lines.covered, lines.count, ours.covered, ours.count
            )),
            None => mismatches.push(format!("{file}: missing from the line export")),
        }
    }
    for file in derived.files.keys() {
        if !reported.files.contains_key(file) {
            mismatches.push(format!("{file}: absent from llvm-cov's summary"));
        }
    }
    if !mismatches.is_empty() {
        let shown = mismatches.len().min(MISMATCH_REPORT_LIMIT);
        bail!(
            "the line export does not reproduce llvm-cov's summary in {} files (covered/mapped); first {shown}:\n  {}",
            mismatches.len(),
            mismatches[..shown].join("\n  ")
        );
    }
    ensure!(
        derived.total == reported.total,
        "the line export totals {}/{} differ from llvm-cov's {}/{} (covered/mapped)",
        derived.total.covered,
        derived.total.count,
        reported.total.covered,
        reported.total.count
    );
    Ok(())
}

/// Union every partition's covered lines per instantiation. Every partition
/// must report the same files, instantiations, groups and mapped lines.
pub fn union<'a>(
    exports: impl IntoIterator<Item = (String, &'a LineExport)>,
) -> Result<LineExport> {
    let mut exports = exports.into_iter();
    let (_, first) = exports.next().context("no line exports to merge")?;
    let mut merged = first.clone();
    for (label, export) in exports {
        ensure!(
            export.files == merged.files,
            "{label} line export covers different source files"
        );
        let mut differences = Vec::new();
        let mut theirs = export.instantiations.iter().peekable();
        for ours in &mut merged.instantiations {
            let key = (ours.file.clone(), ours.name.clone());
            let before = |other: &&Instantiation| (&other.file, &other.name) < (&key.0, &key.1);
            while let Some(other) = theirs.next_if(before) {
                differences.push(format!("{}: {} only in {label}", other.file, other.name));
            }
            match theirs.next_if(|other| other.file == key.0 && other.name == key.1) {
                Some(other)
                    if (other.line, other.column) == (ours.line, ours.column)
                        && other.mapped == ours.mapped =>
                {
                    ours.covered.union(&other.covered);
                }
                Some(_) => differences.push(format!(
                    "{}: {} maps different lines or starts elsewhere in {label}",
                    ours.file, ours.name
                )),
                None => {
                    differences.push(format!("{}: {} missing from {label}", ours.file, ours.name))
                }
            }
        }
        for other in theirs {
            differences.push(format!("{}: {} only in {label}", other.file, other.name));
        }
        if !differences.is_empty() {
            let shown = differences.len().min(MISMATCH_REPORT_LIMIT);
            bail!(
                "{label} line export disagrees on {} instantiations; first {shown}:\n  {}",
                differences.len(),
                differences[..shown].join("\n  ")
            );
        }
    }
    Ok(merged)
}

/// Whether covered / count reaches [`GATE_PERCENT`], exactly as
/// cargo-llvm-cov's floating-point `percent < fail_under_lines` refusal
/// decides for integer counts. No lines never passes.
pub fn passes_gate(total: &Lines) -> bool {
    total.count > 0
        && u128::from(total.covered) * 100 >= u128::from(total.count) * u128::from(GATE_PERCENT)
}

/// Percentage with two decimals, truncated, for logs only.
pub fn percent(total: &Lines) -> String {
    if total.count == 0 {
        return "0.00".to_owned();
    }
    let hundredths = u128::from(total.covered) * 10_000 / u128::from(total.count);
    format!("{}.{:02}", hundredths / 100, hundredths % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/coverage-lines");

    fn fixture(name: &str) -> LlvmExport {
        LlvmExport::read(&Path::new(FIXTURES).join(name)).unwrap()
    }

    /// Derive a fixture's line export and hold it to its own summary. The
    /// rustc fixtures also pass the partition export unchanged; the clang
    /// fixtures carry the region kinds it refuses.
    fn reproduce(export: &str, summary: &str, root: &str) -> LineExport {
        let lines = fixture(export).line_export(root).unwrap();
        let partition = fixture(export).partition_line_export(root);
        if export.starts_with("clang-") {
            let error = format!("{:#}", partition.unwrap_err());
            assert!(error.contains("not a counted code region"), "{error}");
        } else {
            assert_eq!(partition.unwrap(), lines, "{export}");
        }
        let reported = fixture(summary).summary_figures(root).unwrap();
        self_check(&lines.figures().unwrap(), &reported).unwrap();
        // The full export's own totals agree with its summary-only twin.
        assert_eq!(
            fixture(export).summary_figures(root).unwrap(),
            reported,
            "{export}"
        );
        lines
    }

    #[test]
    fn clang_exports_with_every_region_kind_are_reproduced_exactly() {
        // Apple clang 21 -fprofile-instr-generate -fcoverage-mapping, exported
        // by the rustc 1.98.1 llvm-cov (LLVM 22.1.8): code, expansion, skipped
        // and gap regions, a header function with two files and template
        // instantiation groups.
        let c = reproduce("clang-t.json", "clang-t.summary.json", "/fixture");
        assert_eq!(
            c.figures().unwrap().total,
            Lines {
                count: 31,
                covered: 26
            }
        );
        let cpp = reproduce("clang-u.json", "clang-u.summary.json", "/fixture");
        let figures = cpp.figures().unwrap();
        assert_eq!(
            figures.total,
            Lines {
                count: 35,
                covered: 32
            }
        );
        assert_eq!(cpp.files, ["h.hpp", "u.cpp"]);
        assert_eq!(cpp.instantiation_count(), 8);
        let groups: BTreeSet<_> = cpp
            .instantiations
            .iter()
            .map(|i| (&i.file, i.line, i.column))
            .collect();
        assert_eq!(groups.len(), 6);
    }

    #[test]
    fn hosted_linux_and_windows_partitions_are_reproduced_exactly() {
        // Partition 1 of 8 of kuru on ubuntu-latest and windows-latest (CI run
        // 36300952644, cargo-llvm-cov 0.9.1, rustc 1.98.1), trimmed to five
        // sources each with their per-file summaries verbatim. Groups whose
        // instantiations map different lines are present on both.
        for (export, root, count, covered, mapped_differs) in [
            (
                "linux-partition",
                "/home/runner/work/kuru/kuru",
                979,
                787,
                7,
            ),
            ("windows-partition", r"D:\a\kuru\kuru", 714, 456, 7),
        ] {
            let lines = reproduce(
                &format!("{export}.json"),
                &format!("{export}.summary.json"),
                root,
            );
            assert_eq!(
                lines.figures().unwrap().total,
                Lines { count, covered },
                "{export}"
            );
            assert!(lines.files.iter().all(|file| file.starts_with("packages/")));
            let mut groups: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
            for instantiation in &lines.instantiations {
                groups
                    .entry((
                        &instantiation.file,
                        instantiation.line,
                        instantiation.column,
                    ))
                    .or_default()
                    .insert(&instantiation.mapped.0);
            }
            assert_eq!(
                groups.values().filter(|mapped| mapped.len() > 1).count(),
                mapped_differs,
                "{export}"
            );
        }
    }

    #[test]
    fn kuru_partitions_union_into_the_summed_profiles_summary() {
        // kuru-core on aarch64-apple-darwin with cargo-llvm-cov 0.9.1: one
        // instrumented build, two disjoint test selections exported alone (a,
        // b) and over their summed profiles (ab), trimmed to three sources.
        let root = "/work/kuru";
        let a = reproduce(
            "macos-kuru-core-a.json",
            "macos-kuru-core-a.summary.json",
            root,
        );
        let b = reproduce(
            "macos-kuru-core-b.json",
            "macos-kuru-core-b.summary.json",
            root,
        );
        let ab = reproduce(
            "macos-kuru-core-ab.json",
            "macos-kuru-core-ab.summary.json",
            root,
        );
        assert_eq!(
            a.figures().unwrap().total,
            Lines {
                count: 587,
                covered: 109
            }
        );
        assert_eq!(
            b.figures().unwrap().total,
            Lines {
                count: 587,
                covered: 562
            }
        );
        let merged = union([("a".to_owned(), &a), ("b".to_owned(), &b)]).unwrap();
        assert_eq!(merged, ab);
        assert_eq!(
            merged.figures().unwrap().total,
            Lines {
                count: 587,
                covered: 570
            }
        );
        // Multi-instantiation groups (the library built with and without
        // cfg(test)) are present and group by location, not by name.
        let mut sizes: BTreeMap<_, usize> = BTreeMap::new();
        for instantiation in &ab.instantiations {
            *sizes
                .entry((
                    &instantiation.file,
                    instantiation.line,
                    instantiation.column,
                ))
                .or_default() += 1;
        }
        assert_eq!(sizes.len(), 59);
        assert_eq!(sizes.values().filter(|size| **size > 1).count(), 55);
    }

    fn instantiation(name: &str, line: u32, mapped: &[u32], covered: &[u32]) -> Instantiation {
        Instantiation {
            file: "src/a.rs".to_owned(),
            name: name.to_owned(),
            line,
            column: 1,
            mapped: LineSet::from_sorted(mapped.to_vec()),
            covered: LineSet::from_sorted(covered.to_vec()),
        }
    }

    fn export(instantiations: Vec<Instantiation>) -> LineExport {
        LineExport {
            schema: LINES_SCHEMA,
            files: vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()],
            instantiations,
        }
    }

    #[test]
    fn the_merge_unions_per_instantiation_before_the_group_maximum() {
        // Two instantiations of one group: partition 1 covers lines 1 and 2
        // of f, partition 2 covers line 3 of g.
        let one = export(vec![
            instantiation("f", 1, &[1, 2, 3], &[1, 2]),
            instantiation("g", 1, &[1, 2, 3], &[]),
        ]);
        let two = export(vec![
            instantiation("f", 1, &[1, 2, 3], &[]),
            instantiation("g", 1, &[1, 2, 3], &[3]),
        ]);
        let merged = union([("1".to_owned(), &one), ("2".to_owned(), &two)]).unwrap();
        let figures = merged.figures().unwrap();
        // The group counts its best instantiation, f with 2 of 3 lines; a
        // union of lines across instantiations would read 3 of 3.
        assert_eq!(
            figures.total,
            Lines {
                count: 3,
                covered: 2
            }
        );
        assert_eq!(
            figures.files["src/a.rs"],
            Lines {
                count: 3,
                covered: 2
            }
        );
        assert_eq!(figures.files["src/b.rs"], Lines::default());
        // A partition covering f's third line lifts the group through f.
        let three = export(vec![
            instantiation("f", 1, &[1, 2, 3], &[3]),
            instantiation("g", 1, &[1, 2, 3], &[]),
        ]);
        let merged = union([
            ("1".to_owned(), &one),
            ("2".to_owned(), &two),
            ("3".to_owned(), &three),
        ])
        .unwrap();
        assert_eq!(
            merged.figures().unwrap().total,
            Lines {
                count: 3,
                covered: 3
            }
        );
    }

    #[test]
    fn groups_are_keyed_by_start_location_and_each_counts_its_maximum() {
        let lines = export(vec![
            instantiation("a", 1, &[1, 2], &[1]),
            // Same name elsewhere would be another file; another location
            // is another group even for a similar name.
            instantiation("b", 5, &[5, 6, 7], &[5, 6, 7]),
            instantiation("c", 5, &[5, 6, 7, 8], &[5]),
        ]);
        let figures = lines.figures().unwrap();
        assert_eq!(
            figures.total,
            Lines {
                count: 6,
                covered: 4
            }
        );
    }

    #[test]
    fn the_group_start_is_the_first_main_file_region_in_record_order() {
        // The second region starts earlier but comes later in the record;
        // llvm-cov groups by the first region it sees.
        let text = r#"{"type":"llvm.coverage.json.export","version":"3.1.0","data":[{"files":[{"filename":"/r/src/a.rs","summary":{"lines":{"count":3,"covered":2}}}],"functions":[{"name":"f","filenames":["/r/src/a.rs"],"regions":[[3,1,4,2,1,0,0,0],[1,1,2,2,0,0,0,0]]}],"totals":{"lines":{"count":3,"covered":2}}}]}"#;
        let export = LlvmExport::parse(text).unwrap();
        let lines = export.line_export("/r").unwrap();
        assert_eq!(
            (lines.instantiations[0].line, lines.instantiations[0].column),
            (3, 1)
        );
        assert_eq!(lines.instantiations[0].mapped, LineSet(vec![1, 2, 3, 4]));
        assert_eq!(lines.instantiations[0].covered, LineSet(vec![3, 4]));
        // The summary here claims 3/2, which the port refuses.
        let error = self_check(
            &lines.figures().unwrap(),
            &export.summary_figures("/r").unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("src/a.rs: llvm-cov 2/3, port 2/4"),
            "{error}"
        );
    }

    #[test]
    fn functions_outside_exported_sources_and_expanded_files_are_skipped() {
        // g's main file is ignored; h expands into file 1, so file 0 is main.
        let text = r#"{"type":"llvm.coverage.json.export","version":"3.1.0","cargo_llvm_cov":{"version":"0.9.1"},"data":[{"files":[{"filename":"/r/a.rs","summary":{"lines":{"count":2,"covered":1}}}],"functions":[{"name":"g","filenames":["/elsewhere/x.rs"],"regions":[[1,1,9,1,1,0,0,0]]},{"name":"h","filenames":["/r/a.rs","/r/m.rs"],"regions":[[1,1,2,9,0,0,0,0],[1,3,1,8,1,0,1,1],[4,1,4,9,1,1,0,0]]}],"totals":{"lines":{"count":2,"covered":1}}}]}"#;
        let export = LlvmExport::parse(text).unwrap();
        let lines = export.line_export("/r").unwrap();
        assert_eq!(lines.files, ["a.rs"]);
        assert_eq!(lines.instantiation_count(), 1);
        self_check(
            &lines.figures().unwrap(),
            &export.summary_figures("/r").unwrap(),
        )
        .unwrap();
        // Only expanded files: no main view, no instantiation.
        let circular = text.replace("[1,1,2,9,0,0,0,0]", "[1,1,2,9,0,1,0,1]");
        let export = LlvmExport::parse(&circular).unwrap();
        assert_eq!(export.line_export("/r").unwrap().instantiation_count(), 0);
    }

    #[test]
    fn malformed_exports_are_refused() {
        let base = r#"{"type":"llvm.coverage.json.export","version":"3.1.0","data":[{"files":[{"filename":"/r/a.rs","summary":{"lines":{"count":1,"covered":1}}}],"functions":[{"name":"f","filenames":["/r/a.rs"],"regions":[[1,1,1,9,1,0,0,0]]}],"totals":{"lines":{"count":1,"covered":1}}}]}"#;
        LlvmExport::parse(base).unwrap().line_export("/r").unwrap();
        for (text, reason) in [
            (base.replace("3.1.0", "3.0.1"), "reproduces"),
            (base.replace("\"data\":[{", "\"data\":[{},{"), "files"),
            (base.replace("[1,1,1,9,1,0,0,0]", "[1,1,1,9,1,0,0,4]"), "region kind 4"),
            (base.replace("[1,1,1,9,1,0,0,0]", "[2,1,1,9,1,0,0,0]"), "ends before"),
            (base.replace("[1,1,1,9,1,0,0,0]", "[1,1,1,9,1,0,7,1]"), "unknown file 7"),
            (
                base.replace("[1,1,1,9,1,0,0,0]", "[4294967296,1,1,9,1,0,0,0]"),
                "out of range",
            ),
            (
                base.replace(
                    r#"{"name":"f","#,
                    r#"{"name":"f","filenames":["/r/a.rs"],"regions":[[1,1,1,9,1,0,0,0]]},{"name":"f","#,
                ),
                "repeats function f",
            ),
            (base.replace("/r/a.rs", "/q/a.rs"), "outside"),
            (base.replace("\"regions\":[[1,1,1,9,1,0,0,0]]", "\"regions\":[]"), "no region"),
            (
                base.replace("[1,1,1,9,1,0,0,0]", "[1,1,1,9,9223372036854775807,0,0,0]"),
                "function f in a.rs: region Region { start: (1, 1), end: (1, 9), count: 9223372036854775807",
            ),
            (
                base.replace("[1,1,1,9,1,0,0,0]", "[1,1,1,9,18446744073709551615,0,0,0]"),
                "negative counter expression",
            ),
        ] {
            let error = LlvmExport::parse(&text)
                .and_then(|export| export.line_export("/r"))
                .unwrap_err();
            let error = format!("{error:#}");
            assert!(error.contains(reason), "{reason}: {error}");
        }
        let summary_only = base.replace(
            r#","functions":[{"name":"f","filenames":["/r/a.rs"],"regions":[[1,1,1,9,1,0,0,0]]}]"#,
            "",
        );
        let error = LlvmExport::parse(&summary_only)
            .unwrap()
            .line_export("/r")
            .unwrap_err();
        assert!(error.to_string().contains("summary-only"), "{error}");
    }

    #[test]
    fn partition_exports_refuse_skipped_and_zero_length_regions() {
        let base = r#"{"type":"llvm.coverage.json.export","version":"3.1.0","data":[{"files":[{"filename":"/r/a.rs","summary":{"lines":{"count":3,"covered":3}}}],"functions":[{"name":"f","filenames":["/r/a.rs"],"regions":[[1,1,3,9,1,0,0,0],[2,1,2,9,1,0,0,0]]}],"totals":{"lines":{"count":3,"covered":3}}}]}"#;
        let export = LlvmExport::parse(base).unwrap();
        assert_eq!(
            export.partition_line_export("/r").unwrap(),
            export.line_export("/r").unwrap()
        );
        for (region, what) in [
            ("[2,1,2,9,0,0,0,2]", "skipped"),
            ("[2,1,2,9,1,0,0,3]", "gap"),
            ("[2,4,2,4,1,0,0,0]", "zero-length code"),
            ("[2,4,2,4,0,0,0,2]", "zero-length skipped"),
        ] {
            let export = LlvmExport::parse(&base.replace("[2,1,2,9,1,0,0,0]", region)).unwrap();
            export.line_export("/r").unwrap();
            let error = format!("{:#}", export.partition_line_export("/r").unwrap_err());
            assert!(
                error.contains("function f in a.rs: region Region")
                    && error.contains("not a counted code region of nonzero length"),
                "{what}: {error}"
            );
        }
    }

    #[test]
    fn the_self_check_refuses_every_disagreement_with_llvm_cov() {
        let derived = Figures {
            files: BTreeMap::from([
                (
                    "a.rs".to_owned(),
                    Lines {
                        count: 4,
                        covered: 2,
                    },
                ),
                (
                    "b.rs".to_owned(),
                    Lines {
                        count: 2,
                        covered: 2,
                    },
                ),
            ]),
            total: Lines {
                count: 6,
                covered: 4,
            },
        };
        self_check(&derived, &derived).unwrap();
        let mut reported = derived.clone();
        reported.files.insert(
            "a.rs".to_owned(),
            Lines {
                count: 4,
                covered: 3,
            },
        );
        let error = self_check(&derived, &reported).unwrap_err().to_string();
        assert!(error.contains("in 1 files"), "{error}");
        assert!(error.contains("a.rs: llvm-cov 3/4, port 2/4"), "{error}");
        let mut reported = derived.clone();
        reported.files.insert("c.rs".to_owned(), Lines::default());
        let error = self_check(&derived, &reported).unwrap_err().to_string();
        assert!(
            error.contains("c.rs: missing from the line export"),
            "{error}"
        );
        let mut reported = derived.clone();
        reported.files.remove("b.rs");
        let error = self_check(&derived, &reported).unwrap_err().to_string();
        assert!(
            error.contains("b.rs: absent from llvm-cov's summary"),
            "{error}"
        );
        let mut reported = derived.clone();
        reported.total.covered = 5;
        let error = self_check(&derived, &reported).unwrap_err().to_string();
        assert!(
            error.contains("totals 4/6 differ from llvm-cov's 5/6"),
            "{error}"
        );
        // Only the first mismatches are listed.
        let many = Figures {
            files: (0..25)
                .map(|index| {
                    (
                        format!("f{index:02}.rs"),
                        Lines {
                            count: 1,
                            covered: 1,
                        },
                    )
                })
                .collect(),
            total: Lines {
                count: 25,
                covered: 25,
            },
        };
        let error = self_check(&Figures::default(), &many)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("in 25 files") && error.contains("first 10"),
            "{error}"
        );
        assert!(
            error.contains("f09.rs") && !error.contains("f10.rs"),
            "{error}"
        );
    }

    #[test]
    fn partitions_must_agree_on_every_instantiations_structure() {
        let base = export(vec![
            instantiation("f", 1, &[1, 2, 3], &[1]),
            instantiation("g", 7, &[7, 8], &[]),
        ]);
        for (other, reason) in [
            (
                export(vec![instantiation("f", 1, &[1, 2, 3], &[1])]),
                "src/a.rs: g missing from 2",
            ),
            (
                export(vec![
                    instantiation("f", 1, &[1, 2, 3], &[1]),
                    instantiation("g", 7, &[7, 8], &[]),
                    instantiation("h", 9, &[9], &[]),
                ]),
                "src/a.rs: h only in 2",
            ),
            (
                export(vec![
                    instantiation("e", 1, &[1], &[]),
                    instantiation("f", 1, &[1, 2, 3], &[1]),
                    instantiation("g", 7, &[7, 8], &[]),
                ]),
                "src/a.rs: e only in 2",
            ),
            (
                export(vec![
                    instantiation("f", 1, &[1, 2], &[1]),
                    instantiation("g", 7, &[7, 8], &[]),
                ]),
                "f maps different lines",
            ),
            (
                export(vec![
                    instantiation("f", 2, &[1, 2, 3], &[1]),
                    instantiation("g", 7, &[7, 8], &[]),
                ]),
                "f maps different lines or starts elsewhere",
            ),
        ] {
            let error = union([("1".to_owned(), &base), ("2".to_owned(), &other)])
                .unwrap_err()
                .to_string();
            assert!(error.contains(reason), "{reason}: {error}");
        }
        let mut other = base.clone();
        other.files.pop();
        let error = union([("1".to_owned(), &base), ("2".to_owned(), &other)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("different source files"), "{error}");
        assert!(union(std::iter::empty()).is_err());
    }

    #[test]
    fn line_exports_render_canonically_and_refuse_malformed_input() {
        let lines = export(vec![
            instantiation("f", 1, &[1, 2, 3, 7], &[2, 3]),
            instantiation("g", 7, &[7, 8], &[]),
        ]);
        let bytes = lines.render().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(
            text.contains(r#""mapped":[[1,3],[7,7]],"covered":[[2,3]]"#),
            "{text}"
        );
        assert_eq!(LineExport::parse(&bytes).unwrap(), lines);
        for (from, to, reason) in [
            ("[[1,3],[7,7]]", "[[1,3],[4,7]]", "not canonical"),
            ("[[1,3],[7,7]]", "[[7,7],[1,3]]", "not canonical"),
            ("[[1,3],[7,7]]", "[[3,1],[7,7]]", "not canonical"),
            (r#""covered":[[2,3]]"#, r#""covered":[[2,4]]"#, "unmapped"),
            (r#""schema":1"#, r#""schema":2"#, "schema"),
            (
                r#""src/a.rs","src/b.rs""#,
                r#""src/b.rs","src/a.rs""#,
                "sorted",
            ),
            (
                r#""src/a.rs","src/b.rs""#,
                r#""/src/a.rs","src/b.rs""#,
                "normalized",
            ),
            (
                r#""src/a.rs","src/b.rs""#,
                r#""src/c.rs","src/d.rs""#,
                "unlisted",
            ),
            (r#""name":"g""#, r#""name":"a""#, "not sorted"),
            (r#""name":"f""#, r#""name":"""#, "unnamed"),
            (r#""schema":1"#, r#""schema":1,"extra":0"#, "unknown field"),
        ] {
            assert!(text.contains(from), "{from}");
            let error = format!(
                "{:#}",
                LineExport::parse(text.replacen(from, to, 1).as_bytes()).unwrap_err()
            );
            assert!(error.contains(reason), "{reason}: {error}");
        }
    }

    #[test]
    fn the_gate_passes_at_exactly_ninety_five_percent_as_cargo_llvm_cov_does() {
        let lines = |count, covered| Lines { count, covered };
        assert_eq!(GATE_PERCENT, 95);
        assert!(passes_gate(&lines(1000, 950)));
        assert!(!passes_gate(&lines(1000, 949)));
        assert!(passes_gate(&lines(10, 10)));
        assert!(!passes_gate(&lines(0, 0)));
        assert!(!passes_gate(&lines(100_001, 95_000)));
        // cargo-llvm-cov compares covered * 100 / count as f64 with 95.0.
        for (count, covered) in [
            (1000_u64, 950_u64),
            (100_001, 95_000),
            (20, 19),
            (100_000, 94_999),
            (7, 6),
            (9, 8),
            (123_457, 111_111),
        ] {
            let float = covered as f64 * 100.0 / count as f64;
            assert_eq!(
                passes_gate(&lines(count, covered)),
                float >= 95.0,
                "{count} {covered}"
            );
        }
        assert_eq!(percent(&lines(1000, 950)), "95.00");
        assert_eq!(percent(&lines(3, 2)), "66.66");
        assert_eq!(percent(&lines(0, 0)), "0.00");
    }
}
