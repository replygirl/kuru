//! Stage derivation from outside observations, and the summary table.
//!
//! A run's open is cut at ordered milestones: its start, the progress lines,
//! and the moments processes and names were first or last seen. Its stages
//! are the differences between consecutive milestones the run showed, so
//! they partition the open: per run they sum to the time to
//! `Memory: ready.`. A milestone a run did not show merges its two
//! neighbouring stages into one, named after both ends. Spans that contain
//! several stages are reported separately as totals, each with its parts.
//!
//! A sampled milestone is stamped with the end of the first tick that showed
//! it, so it is late by at most one bracket (the start of the previous tick
//! to the end of this one). A stage between two sampled milestones is off by
//! less than one bracket either way, and each stage keeps its bounds. A
//! negative stage means two milestones fell within one bracket, seen in the
//! other order. Nothing here compares a time with a budget.

use std::{collections::BTreeMap, fmt::Write as _};

use serde::{Deserialize, Serialize};

use super::{
    Case, Line, Record,
    observe::{FileSpan, Observation, ProcessSpan, Role, Store, round},
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

/// A moment of the open. Exact for the start and progress lines; for a
/// sampled one, the event happened after `after_ms` and by `at_ms`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Milestone {
    pub name: String,
    pub at_ms: f64,
    pub after_ms: f64,
}

/// The time between two consecutive milestones, with the bounds their
/// brackets allow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub name: String,
    pub ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

/// A span of consecutive stages; it equals the sum of `parts`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Total {
    pub name: String,
    pub ms: f64,
    pub parts: Vec<String>,
}

/// A process lifetime: a lower bound between two sightings (absent when it
/// was seen once) and an upper bound from the tick before it to the tick
/// after it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Lifetime {
    pub lower_ms: Option<f64>,
    pub upper_ms: Option<f64>,
    pub samples: u64,
}

impl Lifetime {
    fn of(span: &ProcessSpan) -> Self {
        Self {
            lower_ms: span.lifetime_lower_ms(),
            upper_ms: span.lifetime_upper_ms(),
            samples: span.samples,
        }
    }
}

/// One `dolt sql-server` process seen on at least two samples.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Engine {
    pub store: Option<Store>,
    pub lifetime: Lifetime,
}

/// Stage times in milliseconds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Stages {
    /// Start to the first progress line: the CLI's work before memory opens.
    pub cli_preamble_ms: Option<f64>,
    /// Start to `Memory: ready.`: the whole open as the user sees it.
    pub ready_ms: Option<f64>,
    /// The milestones this run showed, in order.
    pub milestones: Vec<Milestone>,
    /// Consecutive differences of `milestones`: with `Memory: ready.` shown,
    /// they sum to `ready_ms`.
    pub partition: Vec<Stage>,
    /// Spans over several consecutive stages.
    pub totals: Vec<Total>,
    /// Every `dolt version` process of the run, first start to last exit.
    pub probe: Option<Lifetime>,
    /// Every engine this run started, in start order.
    pub engines: Vec<Engine>,
    /// `Memory: ready.` to exit: the demo turn and the command's close.
    pub turn_and_exit_ms: Option<f64>,
}

impl Stages {
    /// The sum of the partition, which equals `ready_ms` when the run
    /// reached `Memory: ready.`.
    pub fn partition_sum_ms(&self) -> f64 {
        round(self.partition.iter().map(|stage| stage.ms).sum())
    }
}

/// Processes and engine runs started during the run (not those it found).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub owners: usize,
    pub supervisors: usize,
    /// Engine processes seen on at least two samples.
    pub engine_starts: usize,
    /// Engine processes seen on one sample only (for example a short-lived
    /// intermediate process beside a real engine).
    pub transient_engine_processes: usize,
    pub staging_engine_starts: usize,
    pub active_engine_starts: usize,
    pub dolt_version: usize,
    /// Appearances of the active store's `sql-server.info`. Staging stores
    /// are not entered, so their markers are not counted.
    pub engine_marker_intervals: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    Some(round(end? - start?))
}

/// Milestones in their causal order; absent ones are skipped.
#[derive(Default)]
struct Chain(Vec<Milestone>);

impl Chain {
    fn exact(&mut self, name: &str, at: Option<f64>) {
        if let Some(at) = at {
            self.0.push(Milestone {
                name: name.to_owned(),
                at_ms: at,
                after_ms: at,
            });
        }
    }

    /// `bracket` is the time the event was first seen and the time before
    /// which it had not happened.
    fn sampled(&mut self, name: &str, bracket: Option<(f64, f64)>) {
        if let Some((at, after)) = bracket {
            self.0.push(Milestone {
                name: name.to_owned(),
                at_ms: at,
                after_ms: after,
            });
        }
    }

    fn at(&self, name: &str) -> Option<usize> {
        self.0.iter().position(|milestone| milestone.name == name)
    }
}

fn started_bracket(span: &ProcessSpan) -> Option<(f64, f64)> {
    Some((span.first_seen_ms, span.not_seen_ms?))
}

fn exited_bracket(span: &ProcessSpan) -> Option<(f64, f64)> {
    Some((span.gone_by_ms?, span.last_seen_ms))
}

fn appeared_bracket(file: &FileSpan) -> Option<(f64, f64)> {
    Some((file.appeared_by_ms, file.absent_at_ms?))
}

fn vanished_bracket(file: &FileSpan) -> Option<(f64, f64)> {
    Some((file.vanished_by_ms?, file.last_present_ms?))
}

/// An engine's supervisor: the latest one not yet paired that was seen no
/// later than the engine.
fn supervisor_of<'a>(
    supervisors: &[&'a ProcessSpan],
    used: &mut [bool],
    engine: &ProcessSpan,
) -> Option<&'a ProcessSpan> {
    let index = supervisors
        .iter()
        .enumerate()
        .filter(|(index, span)| !used[*index] && span.first_seen_ms <= engine.first_seen_ms)
        .map(|(index, _)| index)
        .next_back()?;
    used[index] = true;
    Some(supervisors[index])
}

/// Named totals: a name and the milestones it spans.
const TOTALS: [(&str, &str, &str); 4] = [
    (
        "owner seen to service endpoint",
        "owner seen",
        "service endpoint",
    ),
    (
        "engine provisioning (install stage lifetime)",
        "install stage appeared",
        "install stage removed",
    ),
    (
        "staged store creation (stage appeared to active store)",
        "store stage appeared",
        "active store appeared",
    ),
    (
        "active open (active supervisor to service endpoint)",
        "active supervisor started",
        "service endpoint",
    ),
];

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
    // The first name matching `matches` that appeared during the run.
    let appeared = |matches: &dyn Fn(&str) -> bool| -> Option<&FileSpan> {
        observation
            .files
            .iter()
            .filter(|span| span.absent_at_ms.is_some() && matches(&span.key))
            .min_by(|left, right| left.appeared_by_ms.total_cmp(&right.appeared_by_ms))
    };
    let owners = started(Role::Owner);
    let (engines, transient): (Vec<&ProcessSpan>, Vec<&ProcessSpan>) = started(Role::SqlServer)
        .into_iter()
        .partition(|span| span.samples >= 2);
    let supervisors = started(Role::Supervisor);
    let probes = started(Role::DoltVersion);
    let mut used = vec![false; supervisors.len()];
    let install = appeared(&|key| key.starts_with("cache/") && key.ends_with("/.install-<tmp>"));
    let activated = appeared(&|key| {
        key.starts_with("cache/")
            && !key.contains(".install-")
            && (key.ends_with("/dolt") || key.ends_with("/dolt.exe"))
    });
    let stage = appeared(&|key| {
        key.starts_with("data/memory/<project>.staging-") && key.matches('/').count() == 2
    });

    let mut chain = Chain::default();
    chain.exact("start", Some(0.0));
    chain.exact(
        "progress line",
        line("Memory: waiting for project ownership"),
    );
    chain.sampled(
        "owner seen",
        owners.first().and_then(|span| started_bracket(span)),
    );
    chain.sampled("install stage appeared", install.and_then(appeared_bracket));
    chain.sampled(
        "`dolt version` started",
        probes.first().and_then(|span| started_bracket(span)),
    );
    chain.sampled(
        "`dolt version` exited",
        probes
            .iter()
            .filter_map(|span| exited_bracket(span))
            .max_by(|left, right| left.0.total_cmp(&right.0)),
    );
    chain.sampled("engine activated", activated.and_then(appeared_bracket));
    chain.sampled("install stage removed", install.and_then(vanished_bracket));
    chain.sampled("store stage appeared", stage.and_then(appeared_bracket));
    let staging: Vec<&ProcessSpan> = engines
        .iter()
        .copied()
        .filter(|span| span.store == Some(Store::Staging))
        .collect();
    let active = engines
        .iter()
        .copied()
        .find(|span| span.store == Some(Store::Active));
    let mut stage_supervisors = Vec::new();
    for (index, engine) in staging.iter().enumerate() {
        let number = index + 1;
        let supervisor = supervisor_of(&supervisors, &mut used, engine);
        stage_supervisors.push(number);
        chain.sampled(
            &format!("staging supervisor {number} started"),
            supervisor.and_then(started_bracket),
        );
        chain.sampled(
            &format!("staging engine {number} started"),
            started_bracket(engine),
        );
        chain.sampled(
            &format!("staging engine {number} exited"),
            exited_bracket(engine),
        );
        chain.sampled(
            &format!("staging supervisor {number} exited"),
            supervisor.and_then(exited_bracket),
        );
    }
    chain.sampled(
        "active store appeared",
        appeared(&|key| key == "data/memory/<project>").and_then(appeared_bracket),
    );
    let active_supervisor =
        active.and_then(|engine| supervisor_of(&supervisors, &mut used, engine));
    chain.sampled(
        "active supervisor started",
        active_supervisor.and_then(started_bracket),
    );
    chain.sampled("active engine started", active.and_then(started_bracket));
    chain.sampled(
        "active engine endpoint",
        appeared(&|key| key == "data/memory/<project>/endpoint.json").and_then(appeared_bracket),
    );
    chain.sampled(
        "service endpoint",
        appeared(&|key| {
            key.starts_with("data/memory/services/") && key.ends_with("/endpoint.json")
        })
        .and_then(appeared_bracket),
    );
    chain.exact("`Memory: ready.`", ready);

    let milestones = chain.0.clone();
    let partition: Vec<Stage> = milestones
        .windows(2)
        .map(|pair| {
            let (from, to) = (&pair[0], &pair[1]);
            Stage {
                name: format!("{} → {}", from.name, to.name),
                ms: round(to.at_ms - from.at_ms),
                min_ms: round(to.after_ms - from.at_ms),
                max_ms: round(to.at_ms - from.after_ms),
            }
        })
        .collect();
    let mut spans: Vec<(String, &str, &str)> = TOTALS
        .iter()
        .map(|(name, from, to)| ((*name).to_owned(), *from, *to))
        .collect();
    let supervisor_names: Vec<(String, String, String)> = stage_supervisors
        .iter()
        .map(|number| {
            (
                format!("staging supervisor {number} lifetime"),
                format!("staging supervisor {number} started"),
                format!("staging supervisor {number} exited"),
            )
        })
        .collect();
    for (name, from, to) in &supervisor_names {
        spans.push((name.clone(), from, to));
    }
    let totals = spans
        .into_iter()
        .filter_map(|(name, from, to)| {
            let (from, to) = (chain.at(from)?, chain.at(to)?);
            (to > from).then(|| Total {
                name,
                ms: round(milestones[to].at_ms - milestones[from].at_ms),
                parts: partition[from..to]
                    .iter()
                    .map(|stage| stage.name.clone())
                    .collect(),
            })
        })
        .collect();
    let stages = Stages {
        cli_preamble_ms: line("Memory: waiting for project ownership")
            .or_else(|| lines.first().map(|line| line.t_ms)),
        ready_ms: ready,
        milestones,
        partition,
        totals,
        probe: (!probes.is_empty()).then(|| {
            let first = probes[0];
            let last = probes
                .iter()
                .max_by(|left, right| {
                    left.gone_by_ms
                        .unwrap_or(f64::INFINITY)
                        .total_cmp(&right.gone_by_ms.unwrap_or(f64::INFINITY))
                })
                .copied()
                .unwrap_or(first);
            if probes.len() == 1 {
                Lifetime::of(first)
            } else {
                Lifetime {
                    lower_ms: between(Some(first.first_seen_ms), Some(last.last_seen_ms))
                        .filter(|_| probes.iter().map(|span| span.samples).sum::<u64>() >= 2),
                    upper_ms: between(first.not_seen_ms, last.gone_by_ms),
                    samples: probes.iter().map(|span| span.samples).sum(),
                }
            }
        }),
        engines: engines
            .iter()
            .map(|span| Engine {
                store: span.store,
                lifetime: Lifetime::of(span),
            })
            .collect(),
        turn_and_exit_ms: between(ready, exit_ms),
    };
    let counts = Counts {
        owners: owners.len(),
        supervisors: supervisors.len(),
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

fn staging(record: &Record, index: usize) -> Option<&Engine> {
    record
        .stages
        .engines
        .iter()
        .filter(|engine| engine.store == Some(Store::Staging))
        .nth(index)
}

type Metric = (&'static str, fn(&Record) -> Option<f64>, bool);

const METRICS: [Metric; 13] = [
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
    ("turn and exit", |r| r.stages.turn_and_exit_ms, false),
    (
        "`dolt version` lifetime, lower bound (runs seen on 2+ samples)",
        |r| r.stages.probe.as_ref().and_then(|probe| probe.lower_ms),
        false,
    ),
    (
        "`dolt version` lifetime, upper bound",
        |r| r.stages.probe.as_ref().and_then(|probe| probe.upper_ms),
        false,
    ),
    (
        "staging engine 1 lifetime, lower bound",
        |r| staging(r, 0).and_then(|engine| engine.lifetime.lower_ms),
        false,
    ),
    (
        "staging engine 2 lifetime, lower bound",
        |r| staging(r, 1).and_then(|engine| engine.lifetime.lower_ms),
        false,
    ),
    (
        "staging engine 3 lifetime, lower bound",
        |r| staging(r, 2).and_then(|engine| engine.lifetime.lower_ms),
        false,
    ),
    (
        "CPU probe (median of its repeats)",
        |r| Some(r.probe.cpu_ms).filter(|value| *value > 0.0),
        false,
    ),
    (
        "IO probe",
        |r| Some(r.probe.io_ms).filter(|value| *value > 0.0),
        false,
    ),
    (
        "open / CPU probe (compare within one label only)",
        |r| {
            r.stages
                .ready_ms
                .filter(|_| r.probe.cpu_ms > 0.0)
                .map(|ready| ready / r.probe.cpu_ms)
        },
        true,
    ),
    (
        "sampling bracket (largest in the run)",
        |r| (r.observation.ticks > 1).then_some(r.observation.max_bracket_ms),
        false,
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

fn row(text: &mut String, cells: &[&str], stats: &Stats, ratio: bool) {
    let cell = |value: f64| {
        if ratio {
            format!("{value:.1}")
        } else {
            format!("{value:.0}")
        }
    };
    let _ = writeln!(
        text,
        "| {} | {} | {} | {} | {} | {} |",
        cells.join(" | "),
        stats.n,
        cell(stats.median),
        cell(stats.min),
        cell(stats.max),
        cell(stats.spread)
    );
}

/// A total's parts as the milestones they pass through.
fn through(parts: &[String]) -> String {
    let mut milestones: Vec<&str> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let mut ends = part.splitn(2, " → ");
        let from = ends.next().unwrap_or_default();
        if index == 0 {
            milestones.push(from);
        }
        milestones.extend(ends.next());
    }
    milestones.join(" → ")
}

/// Named samples in first-seen order.
fn grouped<K: PartialEq>(named: impl Iterator<Item = (K, f64)>) -> Vec<(K, Vec<f64>)> {
    let mut groups: Vec<(K, Vec<f64>)> = Vec::new();
    for (name, value) in named {
        match groups.iter_mut().find(|(group, _)| *group == name) {
            Some((_, values)) => values.push(value),
            None => groups.push((name, vec![value])),
        }
    }
    groups
}

fn describe(records: &[Record]) -> String {
    let Some(first) = records.first() else {
        return "No run completed.".into();
    };
    let observed = if first.mode.files {
        "processes and file names"
    } else {
        "processes only (control series: no file is listed)"
    };
    let cost = records
        .iter()
        .map(|record| record.observation.mean_tick_cost_ms)
        .sum::<f64>()
        / records.len() as f64;
    let bracket = records
        .iter()
        .map(|record| record.observation.max_bracket_ms)
        .fold(0.0_f64, f64::max);
    let between = if first.mode.retire_wait {
        "After the first launch and the warm reopen the harness waits for the memory owner \
         to retire, polling every 100 ms (bound 120 s)."
    } else {
        "Ramp series: no run waits for the owner to retire, so owners accumulate; one final \
         wait (polling every 100 ms, bound 120 s) follows the last run."
    };
    format!(
        "Observer: {observed}, sampled every {:.0} ms (the sampler sleeps for the rest of each \
         period); mean tick cost {cost:.1} ms; largest recorded bracket {bracket:.0} ms. Every \
         sampled milestone is stamped at the end of the tick that first showed it and happened \
         within that bracket, so a stage between two sampled milestones is off by less than \
         {bracket:.0} ms either way, and a process shorter than one period can be missed. \
         {between}",
        first.mode.interval_ms
    )
}

/// The Markdown summary: per case the partition of the open, the totals
/// with their parts, other times, every open sample, and structural counts
/// and the census.
pub fn summary(records: &[Record], label: &str) -> String {
    let mut text = String::new();
    let failed = records.iter().filter(|record| !record.opened()).count();
    let after = records
        .iter()
        .filter(|record| record.opened() && !record.outcome.success)
        .count();
    let _ = writeln!(text, "## Kuru open time: {label} (report only)\n");
    let _ = writeln!(
        text,
        "Release binary measured from outside, {} runs; {failed} of {} runs failed to open \
         (no `Memory: ready.`); {after} commands failed after opening. No budget is applied. \
         Times are milliseconds from process start; ratios are unitless.\n",
        records.len(),
        records.len()
    );
    let _ = writeln!(text, "{}\n", describe(records));

    let worst = records
        .iter()
        .filter_map(|record| {
            record
                .stages
                .ready_ms
                .map(|ready| (record.stages.partition_sum_ms() - ready).abs())
        })
        .fold(0.0_f64, f64::max);
    let _ = writeln!(
        text,
        "### Partition of the open\n\nConsecutive stages between the milestones each run \
         showed. Per run they sum to that run's open (largest difference here: {worst:.1} ms); \
         their medians need not. A stage named after two non-adjacent milestones merges what a \
         run did not show.\n"
    );
    let _ = writeln!(text, "| case | stage | n | median | min | max | spread |");
    let _ = writeln!(text, "|---|---|---|---|---|---|---|");
    for case in Case::ALL {
        let cases: Vec<&Record> = records
            .iter()
            .filter(|record| record.case == case && record.opened())
            .collect();
        let named = cases.iter().flat_map(|record| {
            record
                .stages
                .partition
                .iter()
                .map(|stage| (stage.name.as_str(), stage.ms))
        });
        for (name, samples) in grouped(named) {
            if let Some(stats) = stats(&samples) {
                row(&mut text, &[case.label(), name], &stats, false);
            }
        }
    }

    let _ = writeln!(
        text,
        "\n### Totals\n\nEach total spans the consecutive stages between the milestones it \
         passes through and equals their sum in every run; totals are not additive with the \
         partition.\n"
    );
    let _ = writeln!(
        text,
        "| case | total | through | n | median | min | max | spread |"
    );
    let _ = writeln!(text, "|---|---|---|---|---|---|---|---|");
    for case in Case::ALL {
        let cases: Vec<&Record> = records
            .iter()
            .filter(|record| record.case == case)
            .collect();
        // A run that did not show every milestone inside a total has other
        // parts, so the parts are part of the row's identity.
        let named = cases.iter().flat_map(|record| {
            record
                .stages
                .totals
                .iter()
                .map(|total| ((total.name.clone(), through(&total.parts)), total.ms))
        });
        for ((name, parts), samples) in grouped(named) {
            if let Some(stats) = stats(&samples) {
                row(&mut text, &[case.label(), &name, &parts], &stats, false);
            }
        }
    }

    let _ = writeln!(text, "\n### Other times\n");
    let _ = writeln!(text, "| case | metric | n | median | min | max | spread |");
    let _ = writeln!(text, "|---|---|---|---|---|---|---|");
    for case in Case::ALL {
        let cases: Vec<&Record> = records
            .iter()
            .filter(|record| record.case == case)
            .collect();
        for (name, value, ratio) in METRICS {
            let samples: Vec<f64> = cases.iter().filter_map(|record| value(record)).collect();
            if let Some(stats) = stats(&samples) {
                row(&mut text, &[case.label(), name], &stats, ratio);
            }
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
                    (Some(ready), false) => format!("{ready:.0} (command failed after opening)"),
                    (None, _) => "failed".into(),
                },
            )
            .collect();
        if !samples.is_empty() {
            let _ = writeln!(text, "- {}: {}", case.label(), samples.join(", "));
        }
    }
    structure(&mut text, records);
    text
}

fn structure(text: &mut String, records: &[Record]) {
    let _ = writeln!(
        text,
        "\n| case | engine starts | one-sample engine processes | owners started | `dolt version` processes | owner path | 1-min load before | tick cost ms |"
    );
    let _ = writeln!(text, "|---|---|---|---|---|---|---|---|");
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
        let cost = cases
            .iter()
            .map(|record| record.observation.mean_tick_cost_ms)
            .fold(0.0_f64, f64::max);
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {} | {} | {load} | ≤{cost:.1} |",
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
        );
    }
    let _ = writeln!(
        text,
        "\nCensus before and after each run (ranges over runs). Processes of this user; ours run from the scratch root.\n"
    );
    let _ = writeln!(
        text,
        "| case | foreign Kuru/Dolt before | ours before → after | open files or handles after (ours) | listening TCP ports after (ours) | staging dirs after | interrupted dirs after | owner retired ms | retire wait ms |"
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
        let ports = if cases
            .iter()
            .all(|record| record.census_after.listening_total().is_some())
        {
            range(
                cases
                    .iter()
                    .filter_map(|record| record.census_after.listening_total()),
            )
        } else {
            "unavailable".into()
        };
        let waits = |value: fn(&Record) -> Option<f64>| {
            let samples: Vec<f64> = cases.iter().filter_map(|record| value(record)).collect();
            stats(&samples).map_or_else(
                || "–".into(),
                |stats| format!("{:.0}–{:.0}", stats.min, stats.max),
            )
        };
        let _ = writeln!(
            text,
            "| {} | {} | {} → {} | {} | {ports} | {} | {} | {} | {} |",
            case.label(),
            range(
                cases
                    .iter()
                    .map(|record| record.census_before.foreign_total())
            ),
            range(cases.iter().map(|record| record.census_before.ours_total())),
            range(cases.iter().map(|record| record.census_after.ours_total())),
            range(
                cases
                    .iter()
                    .map(|record| record.census_after.open_files_total())
            ),
            range(
                cases
                    .iter()
                    .map(|record| record.census_after.staging_directories)
            ),
            range(
                cases
                    .iter()
                    .map(|record| record.census_after.interrupted_directories)
            ),
            waits(|record| record.owner_retired_ms),
            waits(|record| record.retire_wait_ms),
        );
    }
}
