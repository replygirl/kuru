//! Usage-scan scaling check: aged fixture stores, gated owner opens and the
//! growth assertion CI runs (`usage-scan-scaling` in `ci.yml`).
//!
//! It is a measurement aid, compiled only with test support on Unix, where
//! a spawned owner inherits `KURU_OPEN_TIMELINE`; the timeline is inert on
//! Windows. Two subcommands of this package's own binary reach it:
//!
//! - `usage-scan-fixture create|seal` (`measure:usage-scan:fixture`): create
//!   one empty project store per size with one ungated owner open, and seal
//!   the stores once `measure:age-store` has aged them. The seal records a
//!   key over the compiled constants that decide what the stores measure, so
//!   a measurement refuses stores sealed by a different build or at another
//!   root. CI ages the fixture in-job on every run and never caches it: even
//!   after `DOLT_GC` the two stores exceed the shared Actions cache budget.
//! - `measure-usage-scan` (`measure:usage-scan`, which alone sets the gate):
//!   per size, one warm-up cycle and then `--samples` cycles. A cycle forces
//!   the full validation, then measures one cold full open and one cold bound
//!   open, each a real spawned owner whose timeline gives the first usage
//!   scan's duration (`usage-pool` to `usage-scan-1`) and its row count.
//!
//! The fixture is aged by this binary, so every write carries the usage
//! validation record and an unforced open of it is bound: it decodes no rows.
//! Before each full open the driver therefore adds one empty commit without a
//! record to the usage branch (`MemoryStore::commit_unrecorded_usage_head`,
//! the head a binary that does not write records would leave). That open finds
//! no record, scans every row and records; the next open, the bound one,
//! decodes none.
//!
//! Layout under the fixture root, whose canonical path is part of the key
//! because the owner binds its scope to the project's canonical path:
//! `project-<n>/` (empty project directories), `data-<n>/` (data
//! directories), `age-<n>.json` (the `age-store` report lines),
//! `create-<n>.log` and `fixture.json` (the seal).
//!
//! The bounds are calibrated from the measured post-paging linear rate; their
//! derivation is [`DERIVATION`] and `docs/development.md`.

use super::aged_store::{Counts, Plan, REPORT_FORMAT, REPORT_FORMAT_VERSION};
use crate::OpenOptions;
use crate::service::EndpointRecord;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fmt::Write as _,
    fs::File,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    time::Duration,
};

/// The fixture's own layout and seal format. Bump it to force a cold rebuild.
pub const FIXTURE_FORMAT_VERSION: u32 = 1;
const KEY_PREFIX: &str = "usage-scan-fixture-v1-";
const KEY_DOMAIN: &str = "kuru.usage-scan-fixture.key";
const SEAL_FORMAT: &str = "kuru.usage-scan-fixture";
const TIMELINE_FORMAT: &str = "kuru.open-timeline";
const TIMELINE_FORMAT_VERSION: u32 = 1;
const TIMELINE_LIMIT: u64 = 64 * 1024;
const SMALL_FILE_LIMIT: u64 = 1024 * 1024;
/// An owner closes, reaps its engine and writes its timeline after its last
/// client releases; this bounds that exit with a named failure.
const EXIT_DEADLINE: Duration = Duration::from_secs(60);
const WARMUPS: usize = 1;
const DEFAULT_SAMPLES: usize = 5;
const MAX_SAMPLES: usize = 99;
const NS_PER_MS: f64 = 1_000_000.0;

/// What the fixture holds: one store per size, each aged with this seed and
/// turn count.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub seed: u64,
    pub turns: u32,
    /// Ascending; the ratio compares the last with the first.
    pub sizes: &'static [u64],
}

impl Spec {
    pub fn plan(&self, conversations: u64) -> Plan {
        Plan {
            seed: self.seed,
            conversations,
            turns: self.turns,
        }
    }

    pub fn expected_rows(&self, conversations: u64) -> u64 {
        Counts::expected(&self.plan(conversations)).usage_rows
    }
}

/// The CI fixture: 1,000 and 5,000 conversations, seed 1, one turn, so
/// 4,000 and 20,000 owned usage rows.
pub const CI: Spec = Spec {
    seed: 1,
    turns: 1,
    sizes: &[1_000, 5_000],
};

/// The growth bounds the check asserts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// The largest allowed `T(largest) / max(T(smallest), floor)`.
    pub ratio: f64,
    /// Below this, the smallest size's time is raised to it for the ratio,
    /// so a few milliseconds of noise cannot fail the check.
    pub floor_ns: u64,
    /// The largest allowed `T(largest)`.
    pub ceiling_ns: u64,
}

/// The calibrated bounds: K = 6 over a 100 ms floor and a 1 s ceiling at the
/// largest size. [`DERIVATION`] is their arithmetic.
pub const CALIBRATED: Bounds = Bounds {
    ratio: 6.0,
    floor_ns: 100_000_000,
    ceiling_ns: 1_000_000_000,
};

/// The label the report prints for [`CALIBRATED`].
pub const CALIBRATED_LABEL: &str = "calibrated";

/// The derivation of [`CALIBRATED`], one line, from the four Ubuntu runs of
/// this check's earlier provisional bounds (every open then scanned, after
/// the primary-key paging): first scans of 4,000 and 20,000 rows took 45.8 and
/// 169.9, 64.3 and 235.8, 51.9 and 177.3, and 56.7 and 212.4 ms, a linear cost
/// of 7.8 to 10.7 us per row over 15 to 21 ms flat and an unfloored ratio of at
/// most 3.75. K = 6 is 1.6 times 3.75 and about a third of the 17 a quadratic
/// scan predicts (21 ms + 25 x 43 ms over 64 ms); every T(4000) is under the
/// 100 ms floor, so the ratio check binds at T(20000) <= 600 ms, 2.5 times the
/// worst 236 ms; the ceiling is 1000 ms, 4.2 times it.
pub const DERIVATION: &str = "derivation: 4 Ubuntu runs, worst linear fit 10.7 us/row + 21 ms flat, T(4000) 46-64 ms, T(20000) 170-236 ms, unfloored ratio at most 3.75 (ideal linear 5, quadratic about 17); K=6 is 1.6x the 3.75, and with T(4000) under the 100 ms floor the ratio check binds at T(20000) <= 600 ms, 2.5x the worst 236 ms; ceiling 1000 ms is 4.2x the worst 236 ms";

// --- Fixture key ---------------------------------------------------------------

/// Every input that decides what a sealed fixture measures.
#[derive(Clone, Copy, Debug)]
pub struct KeyInputs<'a> {
    pub fixture_format: u32,
    pub seed: u64,
    pub turns: u32,
    pub sizes: &'a [u64],
    pub report_format: u32,
    pub schema: i32,
    pub usage_schema: i32,
    pub engine_version: &'a str,
    pub archive_sha256: &'a str,
    pub executable_sha256: &'a str,
    pub target: &'a str,
    pub root: &'a Path,
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

/// `usage-scan-fixture-v1-<sha256>` over length-framed inputs.
pub fn compose_key(inputs: &KeyInputs<'_>) -> String {
    let mut hash = Sha256::new();
    frame(&mut hash, KEY_DOMAIN.as_bytes());
    frame(&mut hash, &inputs.fixture_format.to_be_bytes());
    frame(&mut hash, &inputs.seed.to_be_bytes());
    frame(&mut hash, &inputs.turns.to_be_bytes());
    frame(&mut hash, &(inputs.sizes.len() as u64).to_be_bytes());
    for size in inputs.sizes {
        frame(&mut hash, &size.to_be_bytes());
    }
    frame(&mut hash, &inputs.report_format.to_be_bytes());
    frame(&mut hash, &inputs.schema.to_be_bytes());
    frame(&mut hash, &inputs.usage_schema.to_be_bytes());
    frame(&mut hash, inputs.engine_version.as_bytes());
    frame(&mut hash, inputs.archive_sha256.as_bytes());
    frame(&mut hash, inputs.executable_sha256.as_bytes());
    frame(&mut hash, inputs.target.as_bytes());
    frame(&mut hash, inputs.root.as_os_str().as_encoded_bytes());
    format!("{KEY_PREFIX}{}", hex(&hash.finalize()))
}

/// This build's key for `spec` at the canonical `root`: the schema versions
/// from the migrations, and the engine version and digests from the bundled
/// asset manifest, all compiled in, so a workflow never repeats them.
pub fn compiled_key(spec: &Spec, root: &Path) -> String {
    let [schema, usage_schema] = crate::store::SCHEMA_VERSIONS;
    compose_key(&KeyInputs {
        fixture_format: FIXTURE_FORMAT_VERSION,
        seed: spec.seed,
        turns: spec.turns,
        sizes: spec.sizes,
        report_format: REPORT_FORMAT_VERSION,
        schema,
        usage_schema,
        engine_version: crate::catalog::DOLT_VERSION,
        archive_sha256: crate::catalog::BUNDLED_ASSET.archive_sha256,
        executable_sha256: crate::catalog::BUNDLED_ASSET.executable_sha256,
        target: crate::catalog::BUNDLED_ASSET.target,
        root,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// --- Layout --------------------------------------------------------------------

/// The fixture root, created when absent and canonical.
#[derive(Clone, Debug)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn open(root: &Path) -> Result<Self> {
        ensure!(root.is_absolute(), "the fixture root must be absolute");
        std::fs::create_dir_all(root)
            .with_context(|| format!("create fixture root {}", root.display()))?;
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The canonical project directory for size `n`, created when absent.
    pub fn project(&self, n: u64) -> Result<PathBuf> {
        let path = self.root.join(format!("project-{n}"));
        std::fs::create_dir_all(&path)?;
        let canonical = path.canonicalize()?;
        ensure!(
            canonical == path,
            "fixture project {} is not canonical",
            path.display()
        );
        Ok(path)
    }

    pub fn data(&self, n: u64) -> PathBuf {
        self.root.join(format!("data-{n}"))
    }

    /// The data directory for size `n`, created owner-private when absent;
    /// the memory open refuses any other mode.
    pub fn private_data(&self, n: u64) -> Result<PathBuf> {
        let data = self.data(n);
        crate::files::private_dir(&data)?;
        Ok(data)
    }

    pub fn age_report(&self, n: u64) -> PathBuf {
        self.root.join(format!("age-{n}.json"))
    }

    fn seal(&self) -> PathBuf {
        self.root.join("fixture.json")
    }
}

/// The project scope the owner requires for a canonical project directory.
pub fn project_scope(project: &Path) -> String {
    format!(
        "project/{}",
        hex(&Sha256::digest(project.as_os_str().as_encoded_bytes()))
    )
}

/// The executable an owner runs, its engine supervisor and engine cache
/// (`None`: the default cache under the data directory).
#[derive(Clone, Debug)]
pub struct Engine {
    pub executable: PathBuf,
    pub supervisor: PathBuf,
    pub cache_dir: Option<PathBuf>,
}

impl Engine {
    fn options(&self, data: PathBuf, scope: String) -> OpenOptions {
        let mut options = OpenOptions::new(data, scope);
        options.config.offline = true;
        options.config.cache_dir = self.cache_dir.clone();
        options.supervisor = Some(self.supervisor.clone());
        options
    }
}

// --- Timeline ------------------------------------------------------------------

/// One gated open, from its owner's timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Sample {
    pub generation: String,
    /// `usage-pool` to `usage-scan-1`: the first owned-state scan, with its
    /// flat working-set and schema checks.
    pub scan1_ns: u64,
    /// `usage-validate` to `usage-scan-2`: the second scan.
    pub scan2_ns: u64,
    /// `usage-pool` to `usage-scan-2`: the whole usage block.
    pub usage_block_ns: u64,
    /// `owner-main` to `endpoint-published`: the whole owner open.
    pub open_ns: u64,
    /// The rows the first scan decoded.
    pub usage_rows: u64,
}

#[derive(Deserialize)]
struct RawTimeline {
    format: String,
    format_version: u32,
    service_generation: String,
    events: Vec<RawStamp>,
    counts: RawCounts,
    dropped: u32,
}

#[derive(Deserialize)]
struct RawStamp {
    event: String,
    ns: u64,
}

#[derive(Deserialize)]
struct RawCounts {
    usage_rows: Option<u64>,
}

/// Parse one existing-store open's timeline, refusing anything that would
/// make its numbers ambiguous.
pub fn parse_timeline(bytes: &[u8]) -> Result<Sample> {
    let raw: RawTimeline = serde_json::from_slice(bytes).context("open timeline is not JSON")?;
    ensure!(
        raw.format == TIMELINE_FORMAT && raw.format_version == TIMELINE_FORMAT_VERSION,
        "open timeline format {} v{} is not {TIMELINE_FORMAT} v{TIMELINE_FORMAT_VERSION}",
        raw.format,
        raw.format_version
    );
    ensure!(
        raw.dropped == 0,
        "open timeline dropped {} stamps",
        raw.dropped
    );
    ensure!(
        raw.events.windows(2).all(|pair| pair[0].ns <= pair[1].ns),
        "open timeline offsets decrease"
    );
    let at = |name: &str| -> Result<u64> {
        let mut found = raw.events.iter().filter(|stamp| stamp.event == name);
        let stamp = found
            .next()
            .with_context(|| format!("open timeline has no {name} event"))?;
        ensure!(
            found.next().is_none(),
            "open timeline repeats {name}: not an existing-store open"
        );
        Ok(stamp.ns)
    };
    let owner = at("owner-main")?;
    let pool = at("usage-pool")?;
    let scan1 = at("usage-scan-1")?;
    let validate = at("usage-validate")?;
    let scan2 = at("usage-scan-2")?;
    let published = at("endpoint-published")?;
    let usage_rows = raw
        .counts
        .usage_rows
        .context("open timeline has no usage row count")?;
    Ok(Sample {
        generation: raw.service_generation,
        scan1_ns: scan1 - pool,
        scan2_ns: scan2 - validate,
        usage_block_ns: scan2 - pool,
        open_ns: published - owner,
        usage_rows,
    })
}

/// The median of an odd, non-empty sample.
pub fn median(values: &[u64]) -> Result<u64> {
    ensure!(
        values.len() % 2 == 1,
        "a median needs an odd, non-empty sample, got {}",
        values.len()
    );
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Ok(sorted[sorted.len() / 2])
}

// --- Evaluation ------------------------------------------------------------------

/// Every open of one size. The full series are opens that had to validate
/// every row (warm-ups are checked for rows but not timed); the bound series
/// are the opens that followed, which find the record and decode no row.
#[derive(Clone, Debug)]
pub struct SizeSamples {
    pub conversations: u64,
    pub expected_rows: u64,
    pub warmups: Vec<Sample>,
    pub samples: Vec<Sample>,
    pub bound: Vec<Sample>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SizeSummary {
    pub conversations: u64,
    pub expected_rows: u64,
    pub scan1_ns: Vec<u64>,
    pub scan1_median_ns: u64,
    pub scan2_median_ns: u64,
    pub usage_block_median_ns: u64,
    pub open_median_ns: u64,
    /// The bound series' first scan interval (the flat checks and the record
    /// read; no row is decoded), reported and not bounded.
    pub bound_scan1_ns: Vec<u64>,
    pub bound_scan1_median_ns: u64,
    pub bound_open_median_ns: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub pass: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Verdict {
    pub bounds: &'static str,
    pub ratio_bound: f64,
    pub floor_ns: u64,
    pub ceiling_ns: u64,
    pub sizes: Vec<SizeSummary>,
    pub ratio: f64,
    pub checks: Vec<Check>,
    pub pass: bool,
}

fn ms(ns: u64) -> f64 {
    ns as f64 / NS_PER_MS
}

/// Evaluate `sizes` (ascending, at least two) against `bounds`.
pub fn evaluate(sizes: &[SizeSamples], bounds: Bounds, label: &'static str) -> Result<Verdict> {
    ensure!(sizes.len() >= 2, "the check needs at least two sizes");
    ensure!(
        sizes
            .windows(2)
            .all(|pair| pair[0].conversations < pair[1].conversations),
        "sizes must ascend"
    );
    let mut summaries = Vec::new();
    let mut mismatches = Vec::new();
    let mut recorded = Vec::new();
    for size in sizes {
        let pick = |field: fn(&Sample) -> u64| size.samples.iter().map(field).collect::<Vec<_>>();
        let scan1 = pick(|sample| sample.scan1_ns);
        summaries.push(SizeSummary {
            conversations: size.conversations,
            expected_rows: size.expected_rows,
            scan1_median_ns: median(&scan1)?,
            scan1_ns: scan1,
            scan2_median_ns: median(&pick(|sample| sample.scan2_ns))?,
            usage_block_median_ns: median(&pick(|sample| sample.usage_block_ns))?,
            open_median_ns: median(&pick(|sample| sample.open_ns))?,
            bound_scan1_median_ns: median(
                &size
                    .bound
                    .iter()
                    .map(|sample| sample.scan1_ns)
                    .collect::<Vec<_>>(),
            )?,
            bound_scan1_ns: size.bound.iter().map(|sample| sample.scan1_ns).collect(),
            bound_open_median_ns: median(
                &size
                    .bound
                    .iter()
                    .map(|sample| sample.open_ns)
                    .collect::<Vec<_>>(),
            )?,
        });
        for (index, sample) in size.warmups.iter().chain(&size.samples).enumerate() {
            if sample.usage_rows != size.expected_rows {
                mismatches.push(format!(
                    "{} conversations open {index} decoded {} rows, expected {}",
                    size.conversations, sample.usage_rows, size.expected_rows
                ));
            }
        }
        for (index, sample) in size.bound.iter().enumerate() {
            if sample.usage_rows != 0 {
                recorded.push(format!(
                    "{} conversations recorded reopen {index} decoded {} rows, expected 0",
                    size.conversations, sample.usage_rows
                ));
            }
        }
    }
    let (first, last) = (&summaries[0], &summaries[summaries.len() - 1]);
    let small = first.scan1_median_ns;
    let large = last.scan1_median_ns;
    let ratio = large as f64 / small.max(bounds.floor_ns) as f64;
    let mut checks = vec![Check {
        name: "rows",
        pass: mismatches.is_empty(),
        detail: if mismatches.is_empty() {
            "every open decoded its size's expected usage rows".into()
        } else {
            format!(
                "usage scan row count differs (a forced full open must decode every owned row): {}",
                mismatches.join("; ")
            )
        },
    }];
    let pass = ratio <= bounds.ratio;
    checks.push(Check {
        name: "ratio",
        pass,
        detail: format!(
            "{} T({} rows)={:.1} ms, T({} rows)={:.1} ms, ratio {ratio:.2} {} {} ({label} bound; floor {:.0} ms)",
            if pass {
                "usage scan grew at most linearly:"
            } else {
                "usage scan grew faster than linear:"
            },
            last.expected_rows,
            ms(large),
            first.expected_rows,
            ms(small),
            if pass { "<=" } else { ">" },
            bounds.ratio,
            ms(bounds.floor_ns),
        ),
    });
    let pass = large <= bounds.ceiling_ns;
    checks.push(Check {
        name: "ceiling",
        pass,
        detail: format!(
            "{} T({} rows)={:.1} ms {} {:.0} ms ({label} bound)",
            if pass {
                "usage scan within its ceiling:"
            } else {
                "usage scan exceeded its ceiling:"
            },
            last.expected_rows,
            ms(large),
            if pass { "<=" } else { ">" },
            ms(bounds.ceiling_ns),
        ),
    });
    // The deterministic half of the validation record: the open that follows
    // a full open finds its record and decodes no usage row at either size.
    checks.push(Check {
        name: "bound-rows",
        pass: recorded.is_empty(),
        detail: if recorded.is_empty() {
            "every recorded reopen decoded 0 usage rows".into()
        } else {
            format!(
                "a recorded reopen decoded usage rows (the validation record did not bind it): {}",
                recorded.join("; ")
            )
        },
    });
    let pass = checks.iter().all(|check| check.pass);
    Ok(Verdict {
        bounds: label,
        ratio_bound: bounds.ratio,
        floor_ns: bounds.floor_ns,
        ceiling_ns: bounds.ceiling_ns,
        sizes: summaries,
        ratio,
        checks,
        pass,
    })
}

fn verdict_word(pass: bool) -> &'static str {
    if pass { "PASS" } else { "FAIL" }
}

/// The plain-text report every run prints.
pub fn render_lines(verdict: &Verdict) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "usage-scan check: {} bounds (K={}, floor={:.0} ms, ceiling={:.0} ms at the largest size); {DERIVATION}",
        verdict.bounds.to_uppercase(),
        verdict.ratio_bound,
        ms(verdict.floor_ns),
        ms(verdict.ceiling_ns)
    );
    let _ = writeln!(
        text,
        "full opens (every owned row decoded):\nsize   rows   scan1 median ms [samples]   scan2 ms  usage block ms  open ms"
    );
    for size in &verdict.sizes {
        let samples = size
            .scan1_ns
            .iter()
            .map(|ns| format!("{:.1}", ms(*ns)))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(
            text,
            "{:<6} {:<6} {:.1} [{samples}]   {:.1}  {:.1}  {:.1}",
            size.conversations,
            size.expected_rows,
            ms(size.scan1_median_ns),
            ms(size.scan2_median_ns),
            ms(size.usage_block_median_ns),
            ms(size.open_median_ns)
        );
    }
    let _ = writeln!(
        text,
        "recorded reopens (no row decoded; timings reported, not bounded):\nsize   scan1 median ms [samples]   open ms"
    );
    for size in &verdict.sizes {
        let samples = size
            .bound_scan1_ns
            .iter()
            .map(|ns| format!("{:.1}", ms(*ns)))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(
            text,
            "{:<6} {:.1} [{samples}]   {:.1}",
            size.conversations,
            ms(size.bound_scan1_median_ns),
            ms(size.bound_open_median_ns)
        );
    }
    for check in &verdict.checks {
        let _ = writeln!(
            text,
            "{:<10} {}  {}",
            check.name,
            verdict_word(check.pass),
            check.detail
        );
    }
    let _ = write!(text, "verdict  {}", verdict_word(verdict.pass));
    text
}

/// The Markdown job summary.
pub fn render_summary(verdict: &Verdict) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "### Usage scan scaling: {} ({} bounds)\n",
        verdict_word(verdict.pass),
        verdict.bounds
    );
    let _ = writeln!(
        text,
        "Bounds are **{}**: ratio K = {}, floor {:.0} ms, ceiling {:.0} ms at the largest size, and 0 usage rows decoded by every recorded reopen. {DERIVATION}\n",
        verdict.bounds,
        verdict.ratio_bound,
        ms(verdict.floor_ns),
        ms(verdict.ceiling_ns)
    );
    let _ = writeln!(
        text,
        "| conversations | usage rows | scan 1 median ms | scan 1 samples ms | scan 2 median ms | usage block median ms | open median ms |"
    );
    let _ = writeln!(text, "|---|---|---|---|---|---|---|");
    for size in &verdict.sizes {
        let samples = size
            .scan1_ns
            .iter()
            .map(|ns| format!("{:.1}", ms(*ns)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            text,
            "| {} | {} | {:.1} | {samples} | {:.1} | {:.1} | {:.1} |",
            size.conversations,
            size.expected_rows,
            ms(size.scan1_median_ns),
            ms(size.scan2_median_ns),
            ms(size.usage_block_median_ns),
            ms(size.open_median_ns)
        );
    }
    let _ = writeln!(
        text,
        "\nRecorded reopens decode no row; their timings are reported, not bounded.\n\n| conversations | bound scan 1 median ms | bound scan 1 samples ms | bound open median ms |\n|---|---|---|---|"
    );
    for size in &verdict.sizes {
        let samples = size
            .bound_scan1_ns
            .iter()
            .map(|ns| format!("{:.1}", ms(*ns)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            text,
            "| {} | {:.1} | {samples} | {:.1} |",
            size.conversations,
            ms(size.bound_scan1_median_ns),
            ms(size.bound_open_median_ns)
        );
    }
    let _ = writeln!(text, "\n| check | result | detail |\n|---|---|---|");
    for check in &verdict.checks {
        let _ = writeln!(
            text,
            "| {} | {} | {} |",
            check.name,
            verdict_word(check.pass),
            check.detail
        );
    }
    text
}

// --- Seal ----------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Sealed {
    format: String,
    format_version: u32,
    key: String,
    root: PathBuf,
    sizes: Vec<SealedSize>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SealedSize {
    conversations: u64,
    project: PathBuf,
    scope: String,
    usage_rows: u64,
    age_elapsed_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct AgeReport {
    pub format: String,
    pub format_version: u32,
    pub seed: u64,
    pub conversations: u64,
    pub turns: u32,
    pub usage_rows: u64,
    pub elapsed_ms: u64,
}

/// The one `kuru.aged-store` line in captured standard output. The mise task
/// that ages a store also prints its dependency's output first (the prepared
/// bundle path), so other lines are skipped, but exactly one report must be
/// present.
pub fn age_report(output: &[u8]) -> Result<AgeReport> {
    let text = std::str::from_utf8(output).context("age-store output is not UTF-8")?;
    let mut reports = text
        .lines()
        .filter_map(|line| serde_json::from_str::<AgeReport>(line.trim()).ok())
        .filter(|report| report.format == REPORT_FORMAT);
    let report = reports.next().context("no kuru.aged-store line")?;
    ensure!(reports.next().is_none(), "several kuru.aged-store lines");
    Ok(report)
}

fn read_small(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .take(SMALL_FILE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= SMALL_FILE_LIMIT,
        "{} is too large",
        path.display()
    );
    Ok(bytes)
}

/// The one project scope stored under `data`, which must be `scope`.
fn require_store(data: &Path, scope: &str, project: &Path) -> Result<()> {
    let scopes = super::managed_store_scopes(data)?;
    ensure!(
        scopes == [scope],
        "fixture data directory {} holds stores {scopes:?}, not the one built for project path {}; \
         a sealed fixture must sit at the root it was built for",
        data.display(),
        project.display()
    );
    Ok(())
}

fn timeline_files(data: &Path, scope: &str) -> Result<Vec<String>> {
    let services = EndpointRecord::directory(data, scope)?;
    let entries = match std::fs::read_dir(&services) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut names = Vec::new();
    for entry in entries {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.starts_with("open-timeline-") {
            names.push(name);
        }
    }
    Ok(names)
}

/// Check each aged store against its `age-store` report and the plan's
/// counts, require that no store holds a timeline, and write `fixture.json`.
pub fn seal(layout: &Layout, spec: &Spec) -> Result<()> {
    let mut sizes = Vec::new();
    for &n in spec.sizes {
        let project = layout.project(n)?;
        let scope = project_scope(&project);
        let data = layout.data(n);
        require_store(&data, &scope, &project)?;
        let names = timeline_files(&data, &scope)?;
        ensure!(
            names.is_empty(),
            "fixture store for {n} conversations holds open timelines {names:?}; build fixtures without KURU_OPEN_TIMELINE"
        );
        let path = layout.age_report(n);
        let report = age_report(&read_small(&path)?)
            .with_context(|| format!("{} holds no age-store report", path.display()))?;
        let expected = Counts::expected(&spec.plan(n));
        ensure!(
            report.format == REPORT_FORMAT
                && report.format_version == REPORT_FORMAT_VERSION
                && (
                    report.seed,
                    report.conversations,
                    report.turns,
                    report.usage_rows
                ) == (
                    expected.seed,
                    expected.conversations,
                    expected.turns,
                    expected.usage_rows
                ),
            "{} does not report the planned ageing of {n} conversations",
            path.display()
        );
        sizes.push(SealedSize {
            conversations: n,
            project,
            scope,
            usage_rows: expected.usage_rows,
            age_elapsed_ms: report.elapsed_ms,
        });
    }
    let sealed = Sealed {
        format: SEAL_FORMAT.into(),
        format_version: FIXTURE_FORMAT_VERSION,
        key: compiled_key(spec, layout.root()),
        root: layout.root().to_path_buf(),
        sizes,
    };
    std::fs::write(layout.seal(), serde_json::to_vec_pretty(&sealed)?)?;
    Ok(())
}

/// Read and check the seal of a sealed fixture.
fn read_seal(layout: &Layout, spec: &Spec) -> Result<Sealed> {
    let path = layout.seal();
    let sealed: Sealed = serde_json::from_slice(&read_small(&path)?)
        .with_context(|| format!("{} is not a usage-scan fixture seal", path.display()))?;
    ensure!(
        sealed.format == SEAL_FORMAT && sealed.format_version == FIXTURE_FORMAT_VERSION,
        "{} has format {} v{}",
        path.display(),
        sealed.format,
        sealed.format_version
    );
    ensure!(
        sealed.root == layout.root(),
        "sealed fixture was built for root {}; this run uses {}",
        sealed.root.display(),
        layout.root().display()
    );
    let key = compiled_key(spec, layout.root());
    ensure!(
        sealed.key == key,
        "sealed fixture key {} differs from this build's {key}",
        sealed.key
    );
    ensure!(
        sealed
            .sizes
            .iter()
            .map(|size| size.conversations)
            .eq(spec.sizes.iter().copied()),
        "sealed fixture sizes differ from the plan"
    );
    Ok(sealed)
}

// --- Owner opens ---------------------------------------------------------------

fn log_tail(log: &Path) -> String {
    let Ok(bytes) = std::fs::read(log) else {
        return format!("(owner log {} unreadable)", log.display());
    };
    let start = bytes.len().saturating_sub(4096);
    format!(
        "owner log {} (last {} bytes):\n{}",
        log.display(),
        bytes.len() - start,
        String::from_utf8_lossy(&bytes[start..])
    )
}

/// Spawn one real owner on `options`, keep its generation, release it and
/// await its exit, which follows its close and its timeline write.
async fn run_owner(
    options: &OpenOptions,
    project: &Path,
    executable: &Path,
    log: &Path,
) -> Result<String> {
    let diagnostic = File::create(log)?;
    let owner = async {
        #[cfg(test)]
        let _gate = crate::spawn_gate::spawning().await;
        crate::service::spawn_logged_owner_fixture(options, project, executable, diagnostic).await
    }
    .await
    .with_context(|| log_tail(log))?;
    let generation = EndpointRecord::read(&options.data_dir, &options.project_scope)
        .and_then(|record| record.context("the owner published no endpoint"))
        .map(|record| record.authority.service_generation);
    let exited = owner.exited(EXIT_DEADLINE).await;
    let generation = generation.with_context(|| log_tail(log))?;
    exited.with_context(|| log_tail(log))?;
    Ok(generation)
}

/// Create the empty store for size `n` with one ungated owner open.
pub async fn create(layout: &Layout, spec: &Spec, n: u64, engine: &Engine) -> Result<()> {
    ensure!(
        spec.sizes.contains(&n),
        "the fixture plans sizes {:?}, not {n}",
        spec.sizes
    );
    let project = layout.project(n)?;
    let scope = project_scope(&project);
    let data = layout.private_data(n)?;
    let existing = super::managed_store_scopes(&data)?;
    ensure!(
        existing.is_empty(),
        "fixture data directory {} already holds a store",
        data.display()
    );
    let options = engine.options(data.clone(), scope.clone());
    let log = layout.root().join(format!("create-{n}.log"));
    run_owner(&options, &project, &engine.executable, &log).await?;
    require_store(&data, &scope, &project)
}

/// One measured open: its sample and its timeline's bytes.
#[derive(Clone, Debug)]
pub struct Opened {
    pub sample: Sample,
    pub timeline: Vec<u8>,
    pub timeline_path: PathBuf,
}

async fn measured_open(
    options: &OpenOptions,
    project: &Path,
    executable: &Path,
    log: &Path,
) -> Result<(Sample, Vec<u8>)> {
    let generation = run_owner(options, project, executable, log).await?;
    let path = EndpointRecord::directory(&options.data_dir, &options.project_scope)?
        .join(crate::open_timeline::file_name(&generation));
    let bytes = crate::files::read_bytes(&path, TIMELINE_LIMIT).with_context(|| {
        format!(
            "owner {generation} wrote no open timeline at {}; the measuring task must set KURU_OPEN_TIMELINE=1",
            path.display()
        )
    })?;
    let sample = parse_timeline(&bytes)
        .with_context(|| format!("{}: {}", path.display(), String::from_utf8_lossy(&bytes)))?;
    ensure!(
        sample.generation == generation,
        "timeline {} names generation {}",
        path.display(),
        sample.generation
    );
    Ok((sample, bytes))
}

/// Add the empty commit that leaves the usage head without a validation
/// record, so that the next open validates every owned row. The store is
/// opened in this process (offline, with the engine's supervisor and cache)
/// under the project owner lock, after any previous owner has released it.
async fn force_full_validation(layout: &Layout, n: u64, engine: &Engine) -> Result<()> {
    let claim = super::aged_store::claim(&layout.data(n)).await?;
    let mut options = OpenOptions::new(claim.data_dir.clone(), claim.scope.clone());
    options.config.offline = true;
    options.config.cache_dir = engine.cache_dir.clone();
    options.supervisor = Some(engine.supervisor.clone());
    let forced = async {
        let memory = crate::MemoryStore::open(options).await?;
        let forced = memory.commit_unrecorded_usage_head().await;
        let closed = memory.close().await;
        forced?;
        closed
    }
    .await;
    let released = claim.lock.release();
    forced?;
    released
}

/// Per size, `warmups` warm-up cycles and then `samples` measured cycles.
/// Each cycle forces the full validation, then runs one gated owner open
/// that must decode every owned row and one more that finds the record it
/// left and decodes none (the warm-up cycles stop after the full open). Each
/// timeline is copied into `evidence` beside its owner log, and one record
/// per open is appended to `evidence/records.jsonl`. The owner timeline gate
/// must reach the spawned owners: the measuring task sets it in the
/// environment they inherit.
pub async fn measure(
    layout: &Layout,
    spec: &Spec,
    engine: &Engine,
    warmups: usize,
    samples: usize,
    evidence: &Path,
) -> Result<(Vec<SizeSamples>, Vec<Opened>)> {
    std::fs::create_dir_all(evidence)?;
    let sealed = read_seal(layout, spec)?;
    let mut records = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(evidence.join("records.jsonl"))?;
    let mut sizes = Vec::new();
    let mut opened = Vec::new();
    for size in &sealed.sizes {
        let n = size.conversations;
        let project = layout.project(n)?;
        ensure!(
            project == size.project && project_scope(&project) == size.scope,
            "sealed fixture was built for project path {}; this run uses {}",
            size.project.display(),
            project.display()
        );
        let data = layout.private_data(n)?;
        require_store(&data, &size.scope, &project)?;
        let options = engine.options(data, size.scope.clone());
        let mut measured = SizeSamples {
            conversations: n,
            expected_rows: spec.expected_rows(n),
            warmups: Vec::new(),
            samples: Vec::new(),
            bound: Vec::new(),
        };
        let mut index = 0;
        for cycle in 0..warmups + samples {
            let warmup = cycle < warmups;
            force_full_validation(layout, n, engine).await?;
            for series in [Series::Full, Series::Bound] {
                if warmup && series == Series::Bound {
                    continue;
                }
                let log = evidence.join(format!("owner-{n}-{index}.log"));
                let (sample, bytes) =
                    measured_open(&options, &project, &engine.executable, &log).await?;
                let timeline_path = evidence.join(format!("timeline-{n}-{index}.json"));
                std::fs::write(&timeline_path, &bytes)?;
                writeln!(
                    records,
                    "{}",
                    json!({
                        "conversations": n,
                        "open": index,
                        "series": series.name(),
                        "warmup": warmup,
                        "sample": sample,
                    })
                )?;
                eprintln!(
                    "measure-usage-scan: {n} conversations open {index} ({}{}): scan1 {:.1} ms, rows {}, open {:.1} ms",
                    series.name(),
                    if warmup { ", warm-up" } else { "" },
                    ms(sample.scan1_ns),
                    sample.usage_rows,
                    ms(sample.open_ns)
                );
                opened.push(Opened {
                    sample: sample.clone(),
                    timeline: bytes,
                    timeline_path,
                });
                match (series, warmup) {
                    (Series::Bound, _) => measured.bound.push(sample),
                    (Series::Full, true) => measured.warmups.push(sample),
                    (Series::Full, false) => measured.samples.push(sample),
                }
                index += 1;
            }
        }
        sizes.push(measured);
    }
    Ok((sizes, opened))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Series {
    Full,
    Bound,
}

impl Series {
    fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Bound => "bound",
        }
    }
}

// --- Command lines -------------------------------------------------------------

/// Parsed `--flag value` pairs and bare switches.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Arguments {
    values: Vec<(String, OsString)>,
    switches: Vec<String>,
}

impl Arguments {
    /// Parse `args`: each of `valued` takes the next word, each of
    /// `switches` stands alone; anything else or a repeat is refused.
    pub fn parse(
        command: &str,
        args: impl IntoIterator<Item = OsString>,
        valued: &[&str],
        switches: &[&str],
    ) -> Result<Self> {
        let mut parsed = Self::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let flag = flag
                .into_string()
                .map_err(|_| anyhow::anyhow!("{command} flags must be UTF-8"))?;
            ensure!(
                !parsed.values.iter().any(|(name, _)| *name == flag)
                    && !parsed.switches.contains(&flag),
                "{command} {flag} given twice"
            );
            if valued.contains(&flag.as_str()) {
                let value = args
                    .next()
                    .with_context(|| format!("{command} {flag} needs a value"))?;
                parsed.values.push((flag, value));
            } else if switches.contains(&flag.as_str()) {
                parsed.switches.push(flag);
            } else {
                bail!("{command} does not accept {flag}");
            }
        }
        Ok(parsed)
    }

    fn value(&self, flag: &str) -> Option<&OsString> {
        self.values
            .iter()
            .find(|(name, _)| name == flag)
            .map(|(_, value)| value)
    }

    pub fn path(&self, command: &str, flag: &str) -> Result<Option<PathBuf>> {
        let Some(value) = self.value(flag) else {
            return Ok(None);
        };
        let path = PathBuf::from(value);
        ensure!(path.is_absolute(), "{command} {flag} must be absolute");
        Ok(Some(path))
    }

    pub fn required_path(&self, command: &str, flag: &str) -> Result<PathBuf> {
        self.path(command, flag)?
            .with_context(|| format!("{command} needs {flag}"))
    }

    pub fn number(&self, command: &str, flag: &str) -> Result<Option<u64>> {
        self.value(flag)
            .map(|value| {
                value
                    .to_str()
                    .and_then(|text| text.parse::<u64>().ok())
                    .with_context(|| format!("{command} {flag} must be a non-negative integer"))
            })
            .transpose()
    }

    pub fn switch(&self, flag: &str) -> bool {
        self.switches.iter().any(|name| name == flag)
    }
}

/// The measurement's own options.
#[derive(Debug, PartialEq, Eq)]
pub struct MeasureArguments {
    pub root: PathBuf,
    pub samples: usize,
    pub assert: bool,
    pub evidence: PathBuf,
    pub summary: Option<PathBuf>,
}

/// Parse `--root <dir> --evidence <dir> [--samples <odd n>] [--assert]
/// [--summary <file>]`.
pub fn parse_measure(args: impl IntoIterator<Item = OsString>) -> Result<MeasureArguments> {
    const COMMAND: &str = "measure-usage-scan";
    let parsed = Arguments::parse(
        COMMAND,
        args,
        &["--root", "--samples", "--evidence", "--summary"],
        &["--assert"],
    )?;
    let samples = parsed
        .number(COMMAND, "--samples")?
        .unwrap_or(DEFAULT_SAMPLES as u64);
    ensure!(
        samples % 2 == 1 && samples <= MAX_SAMPLES as u64,
        "{COMMAND} --samples must be odd and at most {MAX_SAMPLES}"
    );
    Ok(MeasureArguments {
        root: parsed.required_path(COMMAND, "--root")?,
        samples: usize::try_from(samples)?,
        assert: parsed.switch("--assert"),
        evidence: parsed.required_path(COMMAND, "--evidence")?,
        summary: parsed.path(COMMAND, "--summary")?,
    })
}

/// The fixture subcommand's mode and options.
#[derive(Debug, PartialEq, Eq)]
pub enum FixtureCommand {
    Create { root: PathBuf, conversations: u64 },
    Seal { root: PathBuf },
}

/// Parse `create --root <dir> --conversations <n>` or `seal --root <dir>`.
pub fn parse_fixture(args: impl IntoIterator<Item = OsString>) -> Result<FixtureCommand> {
    const COMMAND: &str = "usage-scan-fixture";
    let mut args = args.into_iter();
    let mode = args
        .next()
        .and_then(|mode| mode.into_string().ok())
        .context("usage-scan-fixture needs create or seal")?;
    Ok(match mode.as_str() {
        "create" => {
            let parsed = Arguments::parse(COMMAND, args, &["--root", "--conversations"], &[])?;
            FixtureCommand::Create {
                root: parsed.required_path(COMMAND, "--root")?,
                conversations: parsed
                    .number(COMMAND, "--conversations")?
                    .context("usage-scan-fixture create needs --conversations")?,
            }
        }
        "seal" => {
            let parsed = Arguments::parse(COMMAND, args, &["--root"], &[])?;
            FixtureCommand::Seal {
                root: parsed.required_path(COMMAND, "--root")?,
            }
        }
        other => bail!("usage-scan-fixture needs create or seal, not {other}"),
    })
}

/// The release tooling's own engine: this executable's prepared snapshot,
/// with the default engine cache under each data directory.
fn cli_engine() -> Result<Engine> {
    let supervisor = super::prepare_supervisor()?;
    Ok(Engine {
        executable: supervisor.clone(),
        supervisor,
        cache_dir: None,
    })
}

/// The `usage-scan-fixture` subcommand.
pub async fn fixture_main(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    match parse_fixture(args)? {
        FixtureCommand::Create {
            root,
            conversations,
        } => {
            ensure!(
                std::env::var_os(crate::open_timeline::ENV).is_none(),
                "usage-scan-fixture create refuses {}: the sealed fixture must hold no timeline",
                crate::open_timeline::ENV
            );
            let layout = Layout::open(&root)?;
            create(&layout, &CI, conversations, &cli_engine()?).await
        }
        FixtureCommand::Seal { root } => seal(&Layout::open(&root)?, &CI),
    }
}

/// The `measure-usage-scan` subcommand: measure, print the report, write the
/// verdict and summary, and with `--assert` fail on any failed check, with
/// every timeline written to standard error.
pub async fn measure_main(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let arguments = parse_measure(args)?;
    ensure!(
        std::env::var_os(crate::open_timeline::ENV).as_deref() == Some(std::ffi::OsStr::new("1")),
        "measure-usage-scan needs {}=1 so its owners record their open timelines",
        crate::open_timeline::ENV
    );
    let layout = Layout::open(&arguments.root)?;
    let sealed = read_seal(&layout, &CI)?;
    let aged = sealed
        .sizes
        .iter()
        .map(|size| format!("{}={} ms", size.conversations, size.age_elapsed_ms))
        .collect::<Vec<_>>()
        .join(" ");
    println!("fixture: {} (ageing recorded at seal: {aged})", sealed.key);
    let (sizes, opened) = measure(
        &layout,
        &CI,
        &cli_engine()?,
        WARMUPS,
        arguments.samples,
        &arguments.evidence,
    )
    .await?;
    let verdict = evaluate(&sizes, CALIBRATED, CALIBRATED_LABEL)?;
    println!("{}", render_lines(&verdict));
    std::fs::write(
        arguments.evidence.join("verdict.json"),
        serde_json::to_vec_pretty(&verdict)?,
    )?;
    if let Some(summary) = &arguments.summary {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(summary)?;
        writeln!(file, "{}", render_summary(&verdict))?;
        writeln!(
            file,
            "Fixture `{}`; ageing recorded at seal: {aged}.",
            sealed.key
        )?;
    }
    if arguments.assert && !verdict.pass {
        for open in &opened {
            eprintln!(
                "{}:\n{}",
                open.timeline_path.display(),
                String::from_utf8_lossy(&open.timeline)
            );
        }
        let failed = verdict
            .checks
            .iter()
            .filter(|check| !check.pass)
            .map(|check| check.detail.as_str())
            .collect::<Vec<_>>();
        bail!("{}", failed.join("\n"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
