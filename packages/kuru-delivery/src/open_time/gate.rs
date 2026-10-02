//! The open-time gate: exact engine starts per case, then two median budgets.
//!
//! Configured from `KURU_OPEN_TIME_*` environment variables, all or none
//! (see [`Settings::gate`]); without them the command is report only. The
//! gate is evaluated over the main series' records, in two phases:
//!
//! 1. Structure, over every case: each case has runs, each run opened (a
//!    readiness signal), and each run's engine starts equal the case's
//!    expectation. The warm reopen's expectation is keyed on the run's owner
//!    path; a path without an expectation is a violation of its own.
//! 2. Only when every structural check passed: the `new-project` median open
//!    to ready, then the `cold-existing` one, each at most its budget in ms.
//!
//! A pure function of the records, so it is tested on recorded runs without a
//! process.

use std::{collections::BTreeMap, fmt::Write as _};

use anyhow::{Context, Result, bail, ensure};

use super::{
    Case, Record,
    report::{OwnerPath, stats},
};

/// What the gate reads of one run.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub case: Case,
    pub iteration: usize,
    pub path: OwnerPath,
    pub engine_starts: usize,
    pub ready_ms: Option<f64>,
}

impl From<&Record> for Run {
    fn from(record: &Record) -> Self {
        Self {
            case: record.case,
            iteration: record.iteration,
            path: record.path,
            engine_starts: record.counts.engine_starts,
            ready_ms: record.stages.ready_ms,
        }
    }
}

/// Expected engine starts per case and the two median budgets.
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    pub first_launch_starts: usize,
    pub cold_existing_starts: usize,
    pub new_project_starts: usize,
    /// Warm-reopen engine starts per owner path. A warm reopen on a path
    /// not listed here is a violation.
    pub warm_reopen_starts: BTreeMap<OwnerPath, usize>,
    pub new_project_budget_ms: u64,
    pub cold_existing_budget_ms: u64,
}

impl Gate {
    /// The cases whose median open to ready is gated, in checking order.
    fn budgets(&self) -> [(Case, u64); 2] {
        [
            (Case::NewProject, self.new_project_budget_ms),
            (Case::ColdExisting, self.cold_existing_budget_ms),
        ]
    }

    fn expected(&self, case: Case) -> String {
        match case {
            Case::FirstLaunch => self.first_launch_starts.to_string(),
            Case::ColdExisting => self.cold_existing_starts.to_string(),
            Case::NewProject => self.new_project_starts.to_string(),
            Case::WarmReopen => {
                let mut entries: Vec<(&str, usize)> = self
                    .warm_reopen_starts
                    .iter()
                    .map(|(path, starts)| (path.label(), *starts))
                    .collect();
                entries.sort_unstable();
                entries
                    .iter()
                    .map(|(path, starts)| format!("{starts} if {path}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }
    }

    fn paths(&self) -> String {
        let mut labels: Vec<&str> = self
            .warm_reopen_starts
            .keys()
            .map(|path| path.label())
            .collect();
        labels.sort_unstable();
        labels.join(" or ")
    }
}

pub const FIRST_LAUNCH_STARTS: &str = "KURU_OPEN_TIME_EXPECT_FIRST_LAUNCH_STARTS";
pub const COLD_EXISTING_STARTS: &str = "KURU_OPEN_TIME_EXPECT_COLD_EXISTING_STARTS";
pub const WARM_REOPEN_STARTS: &str = "KURU_OPEN_TIME_EXPECT_WARM_REOPEN_STARTS";
pub const NEW_PROJECT_STARTS: &str = "KURU_OPEN_TIME_EXPECT_NEW_PROJECT_STARTS";
pub const NEW_PROJECT_BUDGET_MS: &str = "KURU_OPEN_TIME_BUDGET_NEW_PROJECT_MS";
pub const COLD_EXISTING_BUDGET_MS: &str = "KURU_OPEN_TIME_BUDGET_COLD_EXISTING_MS";

/// The gate's settings as given, each optional.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub first_launch_starts: Option<usize>,
    pub cold_existing_starts: Option<usize>,
    pub new_project_starts: Option<usize>,
    /// `<owner path>=<engine starts>`, comma separated, for example
    /// `attached=0,spawned-owner=1`.
    pub warm_reopen_starts: Option<String>,
    pub new_project_budget_ms: Option<u64>,
    pub cold_existing_budget_ms: Option<u64>,
}

impl Settings {
    /// No setting: report only. Every setting: the gate. Some but not all is
    /// an error, so a missing variable can never drop a check silently.
    pub fn gate(self) -> Result<Option<Gate>> {
        let given = [
            (FIRST_LAUNCH_STARTS, self.first_launch_starts.is_some()),
            (COLD_EXISTING_STARTS, self.cold_existing_starts.is_some()),
            (WARM_REOPEN_STARTS, self.warm_reopen_starts.is_some()),
            (NEW_PROJECT_STARTS, self.new_project_starts.is_some()),
            (NEW_PROJECT_BUDGET_MS, self.new_project_budget_ms.is_some()),
            (
                COLD_EXISTING_BUDGET_MS,
                self.cold_existing_budget_ms.is_some(),
            ),
        ];
        let missing: Vec<&str> = given
            .iter()
            .filter(|(_, set)| !set)
            .map(|(name, _)| *name)
            .collect();
        if missing.len() == given.len() {
            return Ok(None);
        }
        ensure!(
            missing.is_empty(),
            "the open-time gate needs every gate variable; missing {}",
            missing.join(", ")
        );
        let (
            Some(first_launch_starts),
            Some(cold_existing_starts),
            Some(new_project_starts),
            Some(warm),
            Some(new_project_budget_ms),
            Some(cold_existing_budget_ms),
        ) = (
            self.first_launch_starts,
            self.cold_existing_starts,
            self.new_project_starts,
            self.warm_reopen_starts,
            self.new_project_budget_ms,
            self.cold_existing_budget_ms,
        )
        else {
            unreachable!("every gate setting was checked present")
        };
        Ok(Some(Gate {
            first_launch_starts,
            cold_existing_starts,
            new_project_starts,
            warm_reopen_starts: warm_reopen(&warm)
                .with_context(|| format!("{WARM_REOPEN_STARTS}={warm:?}"))?,
            new_project_budget_ms,
            cold_existing_budget_ms,
        }))
    }
}

fn warm_reopen(text: &str) -> Result<BTreeMap<OwnerPath, usize>> {
    let mut starts = BTreeMap::new();
    for entry in text.split(',') {
        let entry = entry.trim();
        let Some((path, count)) = entry.split_once('=') else {
            bail!("expected <owner path>=<engine starts>, found {entry:?}");
        };
        let Some(path) = OwnerPath::parse(path.trim()) else {
            bail!(
                "unknown owner path {path:?}; expected one of {}",
                OwnerPath::ALL.map(OwnerPath::label).join(", ")
            );
        };
        let count: usize = count
            .trim()
            .parse()
            .with_context(|| format!("engine starts for {}", path.label()))?;
        ensure!(
            starts.insert(path, count).is_none(),
            "{} is given twice",
            path.label()
        );
    }
    Ok(starts)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Violation {
    /// The series recorded no run of a case.
    MissingCase { case: Case },
    /// Runs without a readiness signal.
    NotOpened { case: Case, iterations: Vec<usize> },
    /// Warm reopens on an owner path without an expectation.
    UnexpectedPath {
        case: Case,
        path: OwnerPath,
        iterations: Vec<usize>,
    },
    /// Runs whose engine starts differ from the expectation; `path` is the
    /// owner path the expectation was keyed on (warm reopen only).
    EngineStarts {
        case: Case,
        path: Option<OwnerPath>,
        observed: usize,
        expected: usize,
        iterations: Vec<usize>,
    },
    /// A median over its budget, with each run over the budget
    /// (iteration, ms to ready).
    Median {
        case: Case,
        median_ms: f64,
        budget_ms: u64,
        over: Vec<(usize, f64)>,
    },
}

fn iterations(iterations: &[usize]) -> String {
    let list: Vec<String> = iterations.iter().map(ToString::to_string).collect();
    let noun = if iterations.len() == 1 {
        "iteration"
    } else {
        "iterations"
    };
    format!("{noun} {}", list.join(", "))
}

impl Violation {
    fn describe(&self, gate: &Gate) -> String {
        match self {
            Self::MissingCase { case } => format!("{}: no run was recorded", case.label()),
            Self::NotOpened {
                case,
                iterations: runs,
            } => format!(
                "{}: no readiness signal (a failed open), in {}",
                case.label(),
                iterations(runs)
            ),
            Self::UnexpectedPath {
                case,
                path,
                iterations: runs,
            } => format!(
                "{}: owner path {}, expected {}, in {}",
                case.label(),
                path.label(),
                gate.paths(),
                iterations(runs)
            ),
            Self::EngineStarts {
                case,
                path,
                observed,
                expected,
                iterations: runs,
            } => {
                let path = path.map_or_else(String::new, |path| format!(" ({})", path.label()));
                format!(
                    "{}{path}: {observed} engine starts, expected {expected}, in {}",
                    case.label(),
                    iterations(runs)
                )
            }
            Self::Median {
                case,
                median_ms,
                budget_ms,
                over,
            } => {
                let runs: Vec<String> = over
                    .iter()
                    .map(|(iteration, ready)| format!("iteration {iteration} {ready:.1} ms"))
                    .collect();
                format!(
                    "{}: median open to ready {median_ms:.0} ms is over the {budget_ms} ms \
                     budget; runs over the budget: {}",
                    case.label(),
                    runs.join(", ")
                )
            }
        }
    }
}

/// A gated median.
#[derive(Clone, Debug, PartialEq)]
pub struct Median {
    pub case: Case,
    pub n: usize,
    pub median_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub budget_ms: u64,
}

impl Median {
    fn within(&self) -> bool {
        self.median_ms <= self.budget_ms as f64
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    /// Structural violations first, then medians over budget.
    pub violations: Vec<Violation>,
    /// Absent when a structural check failed: the medians were not checked.
    pub medians: Option<Vec<Median>>,
}

impl Verdict {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    /// One line per violation.
    pub fn describe(&self, gate: &Gate) -> String {
        self.violations
            .iter()
            .map(|violation| violation.describe(gate))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn structure(runs: &[&Run], case: Case, gate: &Gate, violations: &mut Vec<Violation>) {
    if runs.is_empty() {
        violations.push(Violation::MissingCase { case });
        return;
    }
    let failed: Vec<usize> = runs
        .iter()
        .filter(|run| run.ready_ms.is_none())
        .map(|run| run.iteration)
        .collect();
    if !failed.is_empty() {
        violations.push(Violation::NotOpened {
            case,
            iterations: failed,
        });
    }
    let mut unexpected: BTreeMap<OwnerPath, Vec<usize>> = BTreeMap::new();
    let mut mismatched: BTreeMap<(Option<OwnerPath>, usize, usize), Vec<usize>> = BTreeMap::new();
    for run in runs {
        let (path, expected) = match case {
            Case::FirstLaunch => (None, gate.first_launch_starts),
            Case::ColdExisting => (None, gate.cold_existing_starts),
            Case::NewProject => (None, gate.new_project_starts),
            Case::WarmReopen => match gate.warm_reopen_starts.get(&run.path) {
                Some(expected) => (Some(run.path), *expected),
                None => {
                    unexpected.entry(run.path).or_default().push(run.iteration);
                    continue;
                }
            },
        };
        if run.engine_starts != expected {
            mismatched
                .entry((path, run.engine_starts, expected))
                .or_default()
                .push(run.iteration);
        }
    }
    for (path, iterations) in unexpected {
        violations.push(Violation::UnexpectedPath {
            case,
            path,
            iterations,
        });
    }
    for ((path, observed, expected), iterations) in mismatched {
        violations.push(Violation::EngineStarts {
            case,
            path,
            observed,
            expected,
            iterations,
        });
    }
}

/// Evaluate the gate over a series' runs.
pub fn evaluate(runs: &[Run], gate: &Gate) -> Verdict {
    let of = |case: Case| -> Vec<&Run> {
        let mut cases: Vec<&Run> = runs.iter().filter(|run| run.case == case).collect();
        cases.sort_by_key(|run| run.iteration);
        cases
    };
    let mut violations = Vec::new();
    for case in Case::ALL {
        structure(&of(case), case, gate, &mut violations);
    }
    if !violations.is_empty() {
        return Verdict {
            violations,
            medians: None,
        };
    }
    let mut medians = Vec::new();
    for (case, budget_ms) in gate.budgets() {
        let cases = of(case);
        let samples: Vec<f64> = cases.iter().filter_map(|run| run.ready_ms).collect();
        // Every run opened and the case has runs: checked above.
        let Some(stats) = stats(&samples) else {
            unreachable!("a gated case has opened runs")
        };
        let median = Median {
            case,
            n: stats.n,
            median_ms: stats.median,
            min_ms: stats.min,
            max_ms: stats.max,
            budget_ms,
        };
        if !median.within() {
            violations.push(Violation::Median {
                case,
                median_ms: stats.median,
                budget_ms,
                over: cases
                    .iter()
                    .filter_map(|run| Some((run.iteration, run.ready_ms?)))
                    .filter(|(_, ready)| *ready > budget_ms as f64)
                    .collect(),
            });
        }
        medians.push(median);
    }
    Verdict {
        violations,
        medians: Some(medians),
    }
}

/// The gate's section of the summary.
pub fn render(verdict: &Verdict, gate: &Gate) -> String {
    let mut text = String::new();
    let result = if verdict.passed() { "passed" } else { "failed" };
    let _ = writeln!(text, "\n## Open-time gate: {result}\n");
    let _ = writeln!(
        text,
        "Checked in order: every case has runs, every run opened, and each run's engine \
         starts equal its case's expectation (the warm reopen's keyed on its owner path); \
         then, only if all of those hold, the median open to ready of `new-project` and \
         then of `cold-existing` against their budgets. First-launch and warm-reopen times \
         are not gated.\n"
    );
    let _ = writeln!(text, "| case | expected engine starts |");
    let _ = writeln!(text, "|---|---|");
    for case in Case::ALL {
        let _ = writeln!(text, "| {} | {} |", case.label(), gate.expected(case));
    }
    match &verdict.medians {
        Some(medians) => {
            let _ = writeln!(
                text,
                "\n| case | n | median ms | min ms | max ms | budget ms | result |"
            );
            let _ = writeln!(text, "|---|---|---|---|---|---|---|");
            for median in medians {
                let _ = writeln!(
                    text,
                    "| {} | {} | {:.0} | {:.0} | {:.0} | {} | {} |",
                    median.case.label(),
                    median.n,
                    median.median_ms,
                    median.min_ms,
                    median.max_ms,
                    median.budget_ms,
                    if median.within() { "within" } else { "over" }
                );
            }
        }
        None => {
            let _ = writeln!(
                text,
                "\nMedians not checked: a run count, readiness or engine start check failed."
            );
        }
    }
    if !verdict.passed() {
        let _ = writeln!(text, "\nViolations:\n");
        for violation in &verdict.violations {
            let _ = writeln!(text, "- {}", violation.describe(gate));
        }
    }
    text
}

#[cfg(test)]
mod tests;
