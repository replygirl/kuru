//! Stage derivation from outside observations, and the summary table.
//!
//! Every stage is a difference between two observed moments. A process or
//! file time carries up to one sampling interval of uncertainty at each end,
//! so a derived stage carries up to two; the record keeps both bracketing
//! ticks. Nothing here compares a time with a budget.

use std::{collections::BTreeMap, fmt::Write as _};

use serde::Serialize;

use super::{
    Case, Line, Record,
    observe::{Observation, ProcessSpan, Role, Store},
};

pub struct Stats {
    pub n: usize,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    pub spread: f64,
}

pub fn stats(samples: &[f64]) -> Option<Stats> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let median = if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    };
    let (min, max) = (sorted[0], sorted[n - 1]);
    Some(Stats {
        n,
        median,
        min,
        max,
        spread: max - min,
    })
}

/// Stage times in milliseconds. Absent when the run did not show both ends.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Stages {
    /// Start to the first progress line: the CLI's work before memory opens.
    pub cli_preamble_ms: Option<f64>,
    /// Start to `Memory: ready.`: the whole open as the user sees it.
    pub ready_ms: Option<f64>,
    /// Start to the first sighting of a new memory owner process.
    pub owner_seen_ms: Option<f64>,
    /// New owner to its first engine: provisioning (extraction, or the warm
    /// hash and version probe) and store preparation.
    pub owner_to_first_engine_ms: Option<f64>,
    /// Engine install stage appearing to the activated executable appearing.
    pub extract_ms: Option<f64>,
    /// Observed lifetime of `dolt version` (a lower bound; may be missed).
    pub probe_seen_ms: Option<f64>,
    /// Every engine this run started, in start order. On a fresh project the
    /// three staging engines initialize, migrate and validate.
    pub engines: Vec<Engine>,
    /// The stage's `ready.json` appearing to the active store appearing.
    pub stage_to_active_ms: Option<f64>,
    /// Active engine start to the owner's published service endpoint:
    /// active validation, candidate recovery, usage ledger and publication.
    pub active_open_ms: Option<f64>,
    /// Service endpoint published to the client's `Memory: ready.` line.
    pub publish_to_ready_ms: Option<f64>,
    /// `Memory: ready.` to exit: the demo turn and the command's close.
    pub turn_and_exit_ms: Option<f64>,
}

/// One `dolt sql-server` process seen on at least two samples.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Engine {
    pub store: Option<Store>,
    /// First to last sighting of the engine process.
    pub lifetime_ms: f64,
    /// Engine process seen to its store's `endpoint.json`, which the
    /// supervisor writes once the engine accepts connections and its startup
    /// checks pass, just before reporting ready.
    pub ready_ms: Option<f64>,
    /// `endpoint.json` present: the owner's work on this engine, until the
    /// supervisor retires the record at stop.
    pub served_ms: Option<f64>,
}

/// Processes and engine runs started during the run (not those it found).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub owners: usize,
    pub supervisors: usize,
    /// Engine processes seen on at least two samples.
    pub engine_starts: usize,
    /// Engine processes seen on one sample only (for example a start retried
    /// after a port collision, or a short-lived intermediate process).
    pub transient_engine_processes: usize,
    pub staging_engine_starts: usize,
    pub active_engine_starts: usize,
    pub dolt_version: usize,
    /// Appearances of an engine's `sql-server.info`: an independent count of
    /// engine runs from files.
    pub engine_marker_intervals: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OwnerPath {
    /// A new owner process started: election, spawn and a full open.
    SpawnedOwner,
    /// No new owner; one was already alive: attach to it.
    Attached,
    #[default]
    NoOwnerObserved,
}

impl OwnerPath {
    fn label(self) -> &'static str {
        match self {
            Self::SpawnedOwner => "spawned-owner",
            Self::Attached => "attached",
            Self::NoOwnerObserved => "no-owner-observed",
        }
    }
}

pub struct Derived {
    pub stages: Stages,
    pub counts: Counts,
    pub path: OwnerPath,
}

fn between(start: Option<f64>, end: Option<f64>) -> Option<f64> {
    Some(((end? - start?) * 10.0).round() / 10.0)
}

fn lifetime(span: &ProcessSpan) -> f64 {
    ((span.last_seen_ms - span.first_seen_ms) * 10.0).round() / 10.0
}

pub fn derive(observation: &Observation, lines: &[Line], exit_ms: Option<f64>) -> Derived {
    let line = |prefix: &str| {
        lines
            .iter()
            .find(|line| line.text.starts_with(prefix))
            .map(|line| line.t_ms)
    };
    let ready = line("Memory: ready.");
    let started = |role: Role| -> Vec<&ProcessSpan> {
        let mut spans: Vec<&ProcessSpan> = observation
            .processes
            .iter()
            .filter(|span| span.role == role && !span.preexisting)
            .collect();
        spans.sort_by(|left, right| left.first_seen_ms.total_cmp(&right.first_seen_ms));
        spans
    };
    // The first appearance during the run of a key matching `matches`.
    let appeared = |matches: &dyn Fn(&str) -> bool| {
        observation
            .files
            .iter()
            .filter(|span| span.absent_at_ms.is_some() && matches(&span.key))
            .map(|span| span.appeared_by_ms)
            .min_by(f64::total_cmp)
    };
    let owners = started(Role::Owner);
    let (engines, transient): (Vec<&ProcessSpan>, Vec<&ProcessSpan>) = started(Role::SqlServer)
        .into_iter()
        .partition(|span| span.last_seen_ms > span.first_seen_ms);
    let staging: Vec<&&ProcessSpan> = engines
        .iter()
        .filter(|span| span.store == Some(Store::Staging))
        .collect();
    let active = engines
        .iter()
        .find(|span| span.store == Some(Store::Active))
        .map(|span| span.first_seen_ms);
    let probes = started(Role::DoltVersion);
    let owner_seen = owners.first().map(|span| span.first_seen_ms);
    let install = appeared(&|key| key.starts_with("cache/") && key.ends_with("/.install-<tmp>"));
    let activated = appeared(&|key| {
        key.starts_with("cache/")
            && !key.contains(".install-")
            && (key.ends_with("/dolt") || key.ends_with("/dolt.exe"))
    });
    let stage_ready = appeared(&|key| {
        key.starts_with("data/memory/") && key.contains(".staging-") && key.ends_with("/ready.json")
    });
    let active_store = appeared(&|key| key == "data/memory/<project>");
    let endpoint = appeared(&|key| {
        key.starts_with("data/memory/services/") && key.ends_with("/endpoint.json")
    });
    let stages = Stages {
        cli_preamble_ms: line("Memory: waiting for project ownership")
            .or_else(|| lines.first().map(|line| line.t_ms)),
        ready_ms: ready,
        owner_seen_ms: owner_seen,
        owner_to_first_engine_ms: between(
            owner_seen,
            engines.first().map(|span| span.first_seen_ms),
        ),
        extract_ms: between(install, activated),
        probe_seen_ms: probes.first().map(|span| lifetime(span)),
        engines: engines
            .iter()
            .map(|span| engine(observation, span))
            .collect(),
        stage_to_active_ms: between(stage_ready, active_store),
        active_open_ms: between(active, endpoint),
        publish_to_ready_ms: between(endpoint, ready),
        turn_and_exit_ms: between(ready, exit_ms),
    };
    let counts = Counts {
        owners: owners.len(),
        supervisors: started(Role::Supervisor).len(),
        engine_starts: engines.len(),
        transient_engine_processes: transient.len(),
        staging_engine_starts: staging.len(),
        active_engine_starts: engines
            .iter()
            .filter(|span| span.store == Some(Store::Active))
            .count(),
        dolt_version: probes.len(),
        engine_marker_intervals: observation
            .files
            .iter()
            .filter(|span| {
                span.absent_at_ms.is_some() && span.key.ends_with("/data/.dolt/sql-server.info")
            })
            .count(),
    };
    let path = if !owners.is_empty() {
        OwnerPath::SpawnedOwner
    } else if observation
        .processes
        .iter()
        .any(|span| span.role == Role::Owner && span.preexisting)
    {
        OwnerPath::Attached
    } else {
        OwnerPath::NoOwnerObserved
    };
    Derived {
        stages,
        counts,
        path,
    }
}

/// Pair an engine with the first store endpoint record of its store kind
/// that appeared while it ran.
fn engine(observation: &Observation, span: &ProcessSpan) -> Engine {
    let staging = span.store == Some(Store::Staging);
    let until = span.gone_by_ms.unwrap_or(f64::INFINITY);
    let endpoint = observation.files.iter().find(|file| {
        file.absent_at_ms.is_some()
            && file.key.starts_with("data/memory/<project>")
            && file.key.ends_with("/endpoint.json")
            && !file.key.starts_with("data/memory/services/")
            && file.key.contains(".staging-") == staging
            && file.appeared_by_ms >= span.first_seen_ms
            && file.appeared_by_ms <= until
    });
    Engine {
        store: span.store,
        lifetime_ms: lifetime(span),
        ready_ms: endpoint
            .and_then(|file| between(Some(span.first_seen_ms), Some(file.appeared_by_ms))),
        served_ms: endpoint
            .and_then(|file| between(Some(file.appeared_by_ms), file.vanished_by_ms)),
    }
}

fn staging(record: &Record, index: usize) -> Option<&Engine> {
    record
        .stages
        .engines
        .iter()
        .filter(|engine| engine.store == Some(Store::Staging))
        .nth(index)
}

fn active(record: &Record) -> Option<&Engine> {
    record
        .stages
        .engines
        .iter()
        .find(|engine| engine.store == Some(Store::Active))
}

type Metric = (&'static str, fn(&Record) -> Option<f64>, bool);

const METRICS: [Metric; 24] = [
    ("open (to `Memory: ready.`)", |r| r.stages.ready_ms, false),
    (
        "exit",
        |r| r.timings.exit_ms.filter(|_| r.outcome.success),
        false,
    ),
    (
        "CLI preamble (to first progress line)",
        |r| r.stages.cli_preamble_ms,
        false,
    ),
    ("start to new owner seen", |r| r.stages.owner_seen_ms, false),
    (
        "owner to first engine",
        |r| r.stages.owner_to_first_engine_ms,
        false,
    ),
    ("engine extraction", |r| r.stages.extract_ms, false),
    ("`dolt version` seen", |r| r.stages.probe_seen_ms, false),
    (
        "staging engine 1 (initialize) lifetime",
        |r| staging(r, 0).map(|e| e.lifetime_ms),
        false,
    ),
    (
        "staging engine 1 spawn to endpoint",
        |r| staging(r, 0).and_then(|e| e.ready_ms),
        false,
    ),
    (
        "staging engine 1 served",
        |r| staging(r, 0).and_then(|e| e.served_ms),
        false,
    ),
    (
        "staging engine 2 (migrate) lifetime",
        |r| staging(r, 1).map(|e| e.lifetime_ms),
        false,
    ),
    (
        "staging engine 2 spawn to endpoint",
        |r| staging(r, 1).and_then(|e| e.ready_ms),
        false,
    ),
    (
        "staging engine 2 served",
        |r| staging(r, 1).and_then(|e| e.served_ms),
        false,
    ),
    (
        "staging engine 3 (validate) lifetime",
        |r| staging(r, 2).map(|e| e.lifetime_ms),
        false,
    ),
    (
        "staging engine 3 spawn to endpoint",
        |r| staging(r, 2).and_then(|e| e.ready_ms),
        false,
    ),
    (
        "staging engine 3 served",
        |r| staging(r, 2).and_then(|e| e.served_ms),
        false,
    ),
    (
        "active engine spawn to endpoint",
        |r| active(r).and_then(|e| e.ready_ms),
        false,
    ),
    (
        "stage ready to active store",
        |r| r.stages.stage_to_active_ms,
        false,
    ),
    (
        "active engine to service endpoint",
        |r| r.stages.active_open_ms,
        false,
    ),
    (
        "service endpoint to `Memory: ready.`",
        |r| r.stages.publish_to_ready_ms,
        false,
    ),
    ("turn and exit", |r| r.stages.turn_and_exit_ms, false),
    (
        "CPU probe",
        |r| Some(r.probe.cpu_ms).filter(|value| *value > 0.0),
        false,
    ),
    (
        "IO probe",
        |r| Some(r.probe.io_ms).filter(|value| *value > 0.0),
        false,
    ),
    (
        "open / CPU probe",
        |r| {
            r.stages
                .ready_ms
                .filter(|_| r.probe.cpu_ms > 0.0)
                .map(|ready| ready / r.probe.cpu_ms)
        },
        true,
    ),
];

fn range(values: impl Iterator<Item = usize>) -> String {
    let values: Vec<usize> = values.collect();
    match (values.iter().min(), values.iter().max()) {
        (Some(min), Some(max)) if min == max => min.to_string(),
        (Some(min), Some(max)) => format!("{min}–{max}"),
        _ => "–".into(),
    }
}

/// The Markdown summary: per case and metric n, median, minimum, maximum and
/// spread; every open sample in run order; structural counts per case.
pub fn summary(records: &[Record], label: &str) -> String {
    let mut text = String::new();
    let failed = records
        .iter()
        .filter(|record| record.stages.ready_ms.is_none())
        .count();
    let _ = writeln!(text, "## Kuru open time: {label} (report only)\n");
    let _ = writeln!(
        text,
        "Release binary measured from outside, {} runs; {failed} of {} runs failed to open. \
         No budget is applied. Times are milliseconds from process start; ratios are unitless. \
         Stage times come from sampled process and file observations and carry up to two \
         sampling intervals of error.\n",
        records.len(),
        records.len()
    );
    let _ = writeln!(text, "| case | metric | n | median | min | max | spread |");
    let _ = writeln!(text, "|---|---|---|---|---|---|---|");
    for case in Case::ALL {
        let cases: Vec<&Record> = records
            .iter()
            .filter(|record| record.case == case)
            .collect();
        for (name, value, ratio) in METRICS {
            let samples: Vec<f64> = cases.iter().filter_map(|record| value(record)).collect();
            let Some(stats) = stats(&samples) else {
                continue;
            };
            let cell = |value: f64| {
                if ratio {
                    format!("{value:.1}")
                } else {
                    format!("{value:.0}")
                }
            };
            let _ = writeln!(
                text,
                "| {} | {name} | {} | {} | {} | {} | {} |",
                case.label(),
                stats.n,
                cell(stats.median),
                cell(stats.min),
                cell(stats.max),
                cell(stats.spread)
            );
        }
    }
    let _ = writeln!(
        text,
        "\nEvery open sample (ms to `Memory: ready.`), in run order:\n"
    );
    for case in Case::ALL {
        let samples: Vec<String> = records
            .iter()
            .filter(|record| record.case == case)
            .map(
                |record| match (record.stages.ready_ms, record.outcome.success) {
                    (Some(ready), true) => format!("{ready:.0}"),
                    (Some(ready), false) => format!("{ready:.0} (command failed)"),
                    (None, _) => "failed".into(),
                },
            )
            .collect();
        if !samples.is_empty() {
            let _ = writeln!(text, "- {}: {}", case.label(), samples.join(", "));
        }
    }
    let _ = writeln!(
        text,
        "\n| case | engine starts | one-sample engine processes | owners started | `dolt version` seen | owner path | foreign Kuru/Dolt processes before | 1-min load before | max sampling gap ms |"
    );
    let _ = writeln!(text, "|---|---|---|---|---|---|---|---|---|");
    for case in Case::ALL {
        let cases: Vec<&Record> = records
            .iter()
            .filter(|record| record.case == case)
            .collect();
        if cases.is_empty() {
            continue;
        }
        let mut paths: BTreeMap<&str, usize> = BTreeMap::new();
        for record in &cases {
            *paths.entry(record.path.label()).or_default() += 1;
        }
        let paths: Vec<String> = paths
            .iter()
            .map(|(path, count)| format!("{path} ×{count}"))
            .collect();
        let loads: Vec<f64> = cases
            .iter()
            .filter_map(|record| record.load.before.map(|load| load[0]))
            .collect();
        let load = stats(&loads).map_or_else(
            || "n/a".into(),
            |stats| format!("{:.2}–{:.2}", stats.min, stats.max),
        );
        let gap = cases
            .iter()
            .map(|record| record.observation.max_gap_ms)
            .fold(0.0_f64, f64::max);
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {} | {} | {} | {load} | {gap:.0} |",
            case.label(),
            range(cases.iter().map(|record| record.counts.engine_starts)),
            range(
                cases
                    .iter()
                    .map(|record| record.counts.transient_engine_processes)
            ),
            range(cases.iter().map(|record| record.counts.owners)),
            range(cases.iter().map(|record| record.counts.dolt_version)),
            paths.join(", "),
            range(
                cases
                    .iter()
                    .map(|record| record.census_before.foreign.values().sum())
            ),
        );
    }
    text
}
