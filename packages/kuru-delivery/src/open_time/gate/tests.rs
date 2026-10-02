//! The gate over recorded post-fix runs: the main series of the two CI runs
//! the budgets derive from, trimmed to the record fields the gate reads.
//! Each failing set changes one field of a passing set.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{Gate, Median, Run, Settings, Violation, evaluate, render, render_titled};
use crate::open_time::{
    Case,
    report::{Counts, OwnerPath, stats},
};

/// Run 36949202478 (head 306bf7fd) and run 36952763676 (head 7dea51fb), both
/// measuring main at 0e562595.
const RUN_A: &str = include_str!("../testdata/gate-36949202478.jsonl");
const RUN_B: &str = include_str!("../testdata/gate-36952763676.jsonl");

#[derive(Deserialize)]
struct Projected {
    case: Case,
    iteration: usize,
    path: OwnerPath,
    counts: Counts,
    stages: Ready,
}

#[derive(Deserialize)]
struct Ready {
    ready_ms: Option<f64>,
}

fn recorded(text: &str) -> Vec<Run> {
    text.lines()
        .map(|line| {
            let run: Projected = serde_json::from_str(line).unwrap();
            Run {
                case: run.case,
                iteration: run.iteration,
                path: run.path,
                engine_starts: run.counts.engine_starts,
                ready_ms: run.stages.ready_ms,
            }
        })
        .collect()
}

/// The gate `ci.yml` configures.
fn ci_gate() -> Gate {
    Gate {
        first_launch_starts: 3,
        cold_existing_starts: 1,
        new_project_starts: 2,
        warm_reopen_starts: BTreeMap::from([
            (OwnerPath::Attached, 0),
            (OwnerPath::SpawnedOwner, 1),
        ]),
        new_project_budget_ms: 890,
        cold_existing_budget_ms: 585,
    }
}

fn run_mut(runs: &mut [Run], case: Case, iteration: usize) -> &mut Run {
    runs.iter_mut()
        .find(|run| run.case == case && run.iteration == iteration)
        .unwrap()
}

fn median_of(medians: &[Median], case: Case) -> &Median {
    medians.iter().find(|median| median.case == case).unwrap()
}

#[test]
fn the_recorded_sets_are_the_post_fix_main_series() {
    for text in [RUN_A, RUN_B] {
        let runs = recorded(text);
        assert_eq!(runs.len(), 40);
        for case in Case::ALL {
            let cases: Vec<&Run> = runs.iter().filter(|run| run.case == case).collect();
            assert_eq!(cases.len(), 10, "{case:?}");
            assert!(cases.iter().all(|run| run.ready_ms.is_some()));
            assert!(cases.iter().all(|run| run.path == OwnerPath::SpawnedOwner));
        }
    }
}

#[test]
fn both_derivation_runs_pass_the_ci_gate() {
    for (text, new_project, cold_existing) in [(RUN_A, 729.75, 399.3), (RUN_B, 731.9, 387.0)] {
        let verdict = evaluate(&recorded(text), &ci_gate());
        assert_eq!(verdict.violations, [], "{verdict:?}");
        assert!(verdict.passed());
        let medians = verdict.medians.as_deref().expect("medians are checked");
        let rows: Vec<Case> = medians.iter().map(|median| median.case).collect();
        assert_eq!(rows, [Case::NewProject, Case::ColdExisting]);
        let row = median_of(medians, Case::NewProject);
        assert!((row.median_ms - new_project).abs() < 1e-9, "{row:?}");
        assert_eq!((row.n, row.budget_ms), (10, 890));
        let row = median_of(medians, Case::ColdExisting);
        assert!((row.median_ms - cold_existing).abs() < 1e-9, "{row:?}");
        assert_eq!((row.n, row.budget_ms), (10, 585));
        let text = render(&verdict, &ci_gate());
        assert!(text.contains("Open-time gate: passed"), "{text}");
        assert!(
            text.contains("| new-project | 10 | 729.8 |")
                || text.contains("| new-project | 10 | 731.9 |"),
            "{text}"
        );
        assert!(text.contains("| 890 | within |"), "{text}");
        assert!(text.contains("| 585 | within |"), "{text}");
    }
}

/// The rule in `ci.yml`: the highest median of the two derivation runs plus
/// three times the largest within-run spread, rounded up to a whole ms.
#[test]
fn the_ci_budgets_follow_from_the_two_derivation_runs() {
    let budget = |case: Case| -> u64 {
        let (mut median, mut spread) = (0.0_f64, 0.0_f64);
        for text in [RUN_A, RUN_B] {
            let samples: Vec<f64> = recorded(text)
                .iter()
                .filter(|run| run.case == case)
                .filter_map(|run| run.ready_ms)
                .collect();
            let stats = stats(&samples).unwrap();
            median = median.max(stats.median);
            spread = spread.max(stats.spread);
        }
        (median + 3.0 * spread).ceil() as u64
    };
    assert_eq!(budget(Case::NewProject), 890);
    assert_eq!(budget(Case::ColdExisting), 585);
    let gate = ci_gate();
    assert_eq!(gate.new_project_budget_ms, budget(Case::NewProject));
    assert_eq!(gate.cold_existing_budget_ms, budget(Case::ColdExisting));
    let workflow = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.github/workflows/ci.yml"
    ))
    .unwrap();
    for line in [
        "KURU_OPEN_TIME_EXPECT_FIRST_LAUNCH_STARTS: \"3\"",
        "KURU_OPEN_TIME_EXPECT_COLD_EXISTING_STARTS: \"1\"",
        "KURU_OPEN_TIME_EXPECT_WARM_REOPEN_STARTS: attached=0,spawned-owner=1",
        "KURU_OPEN_TIME_EXPECT_NEW_PROJECT_STARTS: \"2\"",
        "KURU_OPEN_TIME_BUDGET_NEW_PROJECT_MS: \"890\"",
        "KURU_OPEN_TIME_BUDGET_COLD_EXISTING_MS: \"585\"",
    ] {
        assert!(workflow.contains(line), "ci.yml lacks {line}");
    }
}

#[test]
fn a_count_mismatch_fails_before_any_median_is_checked() {
    for (case, expected) in [
        (Case::FirstLaunch, 3),
        (Case::ColdExisting, 1),
        (Case::NewProject, 2),
    ] {
        let mut runs = recorded(RUN_B);
        // The pre-template new project started 4 engines.
        run_mut(&mut runs, case, 3).engine_starts = 4;
        // Far over budget, yet not reported: medians wait for the counts.
        for run in runs.iter_mut().filter(|run| run.case == Case::NewProject) {
            run.ready_ms = Some(5_000.0);
        }
        let verdict = evaluate(&runs, &ci_gate());
        assert_eq!(
            verdict.violations,
            [Violation::EngineStarts {
                case,
                path: None,
                observed: 4,
                expected,
                iterations: vec![3],
            }]
        );
        assert!(!verdict.passed());
        assert_eq!(verdict.medians, None);
        let text = render(&verdict, &ci_gate());
        assert!(text.contains("Open-time gate: failed"), "{text}");
        assert!(
            text.contains(&format!(
                "{}: 4 engine starts, expected {expected}, in iteration 3",
                case.label()
            )),
            "{text}"
        );
        assert!(text.contains("Medians not checked"), "{text}");
        assert!(!text.contains("over the"), "{text}");
    }
}

#[test]
fn a_new_project_median_over_budget_fails_and_names_the_runs_over_it() {
    let mut runs = recorded(RUN_B);
    for run in runs.iter_mut().filter(|run| run.case == Case::NewProject) {
        run.ready_ms = run.ready_ms.map(|ready| ready + 200.0);
    }
    let verdict = evaluate(&runs, &ci_gate());
    assert!(!verdict.passed());
    let [
        Violation::Median {
            case: Case::NewProject,
            median_ms,
            budget_ms: 890,
            over,
        },
    ] = verdict.violations.as_slice()
    else {
        panic!("{verdict:?}");
    };
    assert!((median_ms - 931.9).abs() < 1e-9, "{median_ms}");
    // 721.3 + 200 and 722.5 + 200 are over 890 too: every run is over it.
    // The runs over the budget are listed in iteration order.
    assert_eq!(over.len(), 10, "{over:?}");
    let iterations: Vec<usize> = over.iter().map(|(iteration, _)| *iteration).collect();
    assert_eq!(iterations, (1..=10).collect::<Vec<_>>());
    assert!(over.iter().all(|(_, ready)| *ready > 890.0));
    let text = render(&verdict, &ci_gate());
    assert!(
        text.contains("new-project: median open to ready 931.9 ms is over the 890 ms budget"),
        "{text}"
    );
    assert!(text.contains("| 890 | over |"), "{text}");
    assert!(text.contains("| 585 | within |"), "{text}");
}

/// A budget equal to a whole-ms median passes; one ms less fails, listing
/// only the runs strictly over it.
fn assert_budget_boundary(runs: &[Run], case: Case, median_ms: u64) {
    let budget = |gate: &mut Gate, budget_ms: u64| match case {
        Case::NewProject => gate.new_project_budget_ms = budget_ms,
        Case::ColdExisting => gate.cold_existing_budget_ms = budget_ms,
        other => panic!("{other:?} has no budget"),
    };
    let mut gate = ci_gate();
    budget(&mut gate, median_ms);
    let verdict = evaluate(runs, &gate);
    assert_eq!(verdict.violations, [], "{verdict:?}");
    let medians = verdict.medians.as_deref().expect("medians are checked");
    let row = median_of(medians, case);
    assert_eq!(row.median_ms, median_ms as f64, "{row:?}");
    assert_eq!(row.budget_ms, median_ms);
    let text = render(&verdict, &gate);
    let row = format!("| {} | 10 | {:.1} |", case.label(), median_ms as f64);
    let line = text
        .lines()
        .find(|line| line.starts_with(&row))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        line.ends_with(&format!("| {median_ms} | within |")),
        "{line}"
    );

    budget(&mut gate, median_ms - 1);
    let verdict = evaluate(runs, &gate);
    let [
        Violation::Median {
            case: violated,
            median_ms: observed,
            budget_ms,
            over,
        },
    ] = verdict.violations.as_slice()
    else {
        panic!("{verdict:?}");
    };
    assert_eq!(
        (*violated, *observed, *budget_ms),
        (case, median_ms as f64, median_ms - 1)
    );
    let mut expected: Vec<(usize, f64)> = runs
        .iter()
        .filter(|run| run.case == case)
        .filter_map(|run| Some((run.iteration, run.ready_ms?)))
        .filter(|(_, ready)| *ready > (median_ms - 1) as f64)
        .collect();
    expected.sort_by_key(|(iteration, _)| *iteration);
    assert_eq!(over, &expected);
    // The runs at the old budget, now one ms over the new one, are listed.
    assert!(
        over.iter().any(|(_, ready)| *ready == median_ms as f64),
        "{over:?}"
    );
}

#[test]
fn a_median_at_the_budget_passes_and_one_ms_under_it_fails() {
    // Run B's recorded cold-existing samples 5 and 6 (in order) are both
    // 387.0 ms (iterations 2, 6 and 10 are 387.0), so the median is exactly
    // 387 ms; eight runs (386.5 ms and up) are over 386.
    let runs = recorded(RUN_B);
    assert_budget_boundary(&runs, Case::ColdExisting, 387);

    // Run B's new-project samples 5 and 6 are iterations 1 (731.4 ms) and
    // 10 (732.4 ms); both at 732.0 keep the order and make the median
    // exactly 732 ms.
    let mut runs = recorded(RUN_B);
    for iteration in [1, 10] {
        run_mut(&mut runs, Case::NewProject, iteration).ready_ms = Some(732.0);
    }
    assert_budget_boundary(&runs, Case::NewProject, 732);
}

#[test]
fn a_cold_existing_median_over_budget_fails_on_its_own() {
    let mut runs = recorded(RUN_A);
    for run in runs.iter_mut().filter(|run| run.case == Case::ColdExisting) {
        run.ready_ms = run.ready_ms.map(|ready| ready + 200.0);
    }
    let verdict = evaluate(&runs, &ci_gate());
    let [
        Violation::Median {
            case: Case::ColdExisting,
            median_ms,
            budget_ms: 585,
            over,
        },
    ] = verdict.violations.as_slice()
    else {
        panic!("{verdict:?}");
    };
    assert!((median_ms - 599.3).abs() < 1e-9, "{median_ms}");
    let iterations: Vec<usize> = over.iter().map(|(iteration, _)| *iteration).collect();
    let mut expected: Vec<usize> = runs
        .iter()
        .filter(|run| run.case == Case::ColdExisting && run.ready_ms.unwrap() > 585.0)
        .map(|run| run.iteration)
        .collect();
    expected.sort_unstable();
    assert_eq!(iterations, expected);
    let text = render(&verdict, &ci_gate());
    assert!(
        text.contains("cold-existing: median open to ready 599.3 ms is over the 585 ms budget"),
        "{text}"
    );
    assert!(text.contains("| 890 | within |"), "{text}");
}

#[test]
fn both_medians_over_budget_are_both_reported_new_project_first() {
    let mut runs = recorded(RUN_A);
    for run in runs
        .iter_mut()
        .filter(|run| matches!(run.case, Case::NewProject | Case::ColdExisting))
    {
        run.ready_ms = run.ready_ms.map(|ready| ready + 400.0);
    }
    let cases: Vec<Case> = evaluate(&runs, &ci_gate())
        .violations
        .iter()
        .map(|violation| match violation {
            Violation::Median { case, .. } => *case,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(cases, [Case::NewProject, Case::ColdExisting]);
}

#[test]
fn a_warm_reopen_is_checked_against_the_count_for_its_owner_path() {
    // A path with no expectation is a violation of its own.
    let mut runs = recorded(RUN_B);
    run_mut(&mut runs, Case::WarmReopen, 4).path = OwnerPath::NoOwnerObserved;
    let verdict = evaluate(&runs, &ci_gate());
    assert_eq!(
        verdict.violations,
        [Violation::UnexpectedPath {
            case: Case::WarmReopen,
            path: OwnerPath::NoOwnerObserved,
            iterations: vec![4],
        }]
    );
    assert_eq!(verdict.medians, None);
    let text = render(&verdict, &ci_gate());
    assert!(
        text.contains(
            "warm-reopen: owner path no-owner-observed, expected attached or spawned-owner, in iteration 4"
        ),
        "{text}"
    );

    // An attach with the spawned owner's engine start is a count mismatch
    // for the attached path.
    let mut runs = recorded(RUN_B);
    run_mut(&mut runs, Case::WarmReopen, 2).path = OwnerPath::Attached;
    let verdict = evaluate(&runs, &ci_gate());
    assert_eq!(
        verdict.violations,
        [Violation::EngineStarts {
            case: Case::WarmReopen,
            path: Some(OwnerPath::Attached),
            observed: 1,
            expected: 0,
            iterations: vec![2],
        }]
    );
    assert!(
        render(&verdict, &ci_gate())
            .contains("warm-reopen (attached): 1 engine starts, expected 0, in iteration 2")
    );

    // An attach that starts no engine passes beside spawned owners.
    run_mut(&mut runs, Case::WarmReopen, 2).engine_starts = 0;
    assert!(evaluate(&runs, &ci_gate()).passed());

    // With only the spawned-owner path expected, an attach is a path
    // mismatch.
    let mut gate = ci_gate();
    gate.warm_reopen_starts.remove(&OwnerPath::Attached);
    assert_eq!(
        evaluate(&runs, &gate).violations,
        [Violation::UnexpectedPath {
            case: Case::WarmReopen,
            path: OwnerPath::Attached,
            iterations: vec![2],
        }]
    );
}

#[test]
fn a_failed_open_or_a_missing_case_fails_even_when_the_counts_match() {
    let mut runs = recorded(RUN_A);
    run_mut(&mut runs, Case::ColdExisting, 2).ready_ms = None;
    let verdict = evaluate(&runs, &ci_gate());
    assert_eq!(
        verdict.violations,
        [Violation::NotOpened {
            case: Case::ColdExisting,
            iterations: vec![2],
        }]
    );
    assert_eq!(verdict.medians, None);
    assert!(
        render(&verdict, &ci_gate())
            .contains("cold-existing: no readiness signal (a failed open), in iteration 2")
    );

    let runs: Vec<Run> = recorded(RUN_A)
        .into_iter()
        .filter(|run| run.case != Case::NewProject)
        .collect();
    let verdict = evaluate(&runs, &ci_gate());
    assert_eq!(
        verdict.violations,
        [Violation::MissingCase {
            case: Case::NewProject
        }]
    );
    assert!(render(&verdict, &ci_gate()).contains("new-project: no run was recorded"));
}

fn complete() -> Settings {
    Settings {
        first_launch_starts: Some(3),
        cold_existing_starts: Some(1),
        new_project_starts: Some(2),
        warm_reopen_starts: Some("attached=0, spawned-owner=1".into()),
        new_project_budget_ms: Some(890),
        cold_existing_budget_ms: Some(585),
    }
}

#[test]
fn the_gate_is_configured_completely_or_not_at_all() {
    assert_eq!(Settings::default().gate().unwrap(), None);
    assert_eq!(complete().gate().unwrap(), Some(ci_gate()));

    let mut partial = complete();
    partial.cold_existing_budget_ms = None;
    let error = format!("{:#}", partial.gate().unwrap_err());
    assert!(
        error.contains("KURU_OPEN_TIME_BUDGET_COLD_EXISTING_MS"),
        "{error}"
    );
    let partial = Settings {
        new_project_budget_ms: Some(890),
        ..Settings::default()
    };
    let error = format!("{:#}", partial.gate().unwrap_err());
    for name in [
        "KURU_OPEN_TIME_EXPECT_FIRST_LAUNCH_STARTS",
        "KURU_OPEN_TIME_EXPECT_COLD_EXISTING_STARTS",
        "KURU_OPEN_TIME_EXPECT_WARM_REOPEN_STARTS",
        "KURU_OPEN_TIME_EXPECT_NEW_PROJECT_STARTS",
        "KURU_OPEN_TIME_BUDGET_COLD_EXISTING_MS",
    ] {
        assert!(error.contains(name), "{error}");
    }

    for bad in [
        "",
        "attached",
        "attached=",
        "attached=x",
        "bogus=1",
        "attached=0,attached=1",
    ] {
        let mut settings = complete();
        settings.warm_reopen_starts = Some(bad.into());
        assert!(settings.gate().is_err(), "{bad:?}");
    }
    let mut settings = complete();
    settings.warm_reopen_starts = Some("no-owner-observed=0".into());
    assert_eq!(
        settings.gate().unwrap().unwrap().warm_reopen_starts,
        BTreeMap::from([(OwnerPath::NoOwnerObserved, 0)])
    );
}

/// Runner noise moves medians, not engine start counts, so only a first
/// series that misses nothing but a median budget is measured again, once.
mod decision {
    use super::*;
    use crate::open_time::gate::{Decision, Judgement, Verdict};

    /// A series' verdict under the CI gate.
    fn verdict(runs: &[Run]) -> Verdict {
        evaluate(runs, &ci_gate())
    }

    fn passing() -> Verdict {
        verdict(&recorded(RUN_A))
    }

    /// New-project runs 200 ms slower: every structural check holds, one
    /// median budget is missed.
    fn median_miss(text: &str) -> Verdict {
        let mut runs = recorded(text);
        for run in runs.iter_mut().filter(|run| run.case == Case::NewProject) {
            run.ready_ms = run.ready_ms.map(|ready| ready + 200.0);
        }
        let verdict = verdict(&runs);
        assert!(verdict.missed_only_a_median(), "{verdict:?}");
        verdict
    }

    /// One new-project run starts the pre-template 4 engines, and every
    /// new-project run is far over its budget too.
    fn count_violation(text: &str) -> Verdict {
        let mut runs = recorded(text);
        run_mut(&mut runs, Case::NewProject, 3).engine_starts = 4;
        for run in runs.iter_mut().filter(|run| run.case == Case::NewProject) {
            run.ready_ms = Some(5_000.0);
        }
        let verdict = verdict(&runs);
        assert!(!verdict.passed() && !verdict.missed_only_a_median());
        verdict
    }

    fn failed_open(text: &str) -> Verdict {
        let mut runs = recorded(text);
        run_mut(&mut runs, Case::ColdExisting, 2).ready_ms = None;
        let verdict = verdict(&runs);
        assert!(!verdict.passed() && !verdict.missed_only_a_median());
        verdict
    }

    #[test]
    fn a_passing_first_series_decides_and_requests_no_second() {
        let decision = Decision::decide(&passing(), None);
        assert_eq!(decision, Decision::Passed);
        assert!(decision.passed());
        assert_eq!(decision.series(), Some(1));
        // A second verdict cannot change a decided first series.
        let second = median_miss(RUN_B);
        assert_eq!(
            Decision::decide(&passing(), Some(&second)),
            Decision::Passed
        );
        assert!(
            decision
                .render()
                .contains("passed, decided by the first series")
        );
    }

    #[test]
    fn a_count_violation_or_failed_open_in_the_first_series_fails_without_a_second() {
        for first in [count_violation(RUN_A), failed_open(RUN_A)] {
            let decision = Decision::decide(&first, None);
            assert_eq!(decision, Decision::Failed, "{first:?}");
            assert!(!decision.passed());
            assert_eq!(decision.series(), Some(1));
            // Even a passing second series is never consulted.
            assert_eq!(Decision::decide(&first, Some(&passing())), Decision::Failed);
            assert!(
                decision
                    .render()
                    .contains("failed, decided by the first series"),
                "{}",
                decision.render()
            );
        }
        // Both medians over budget is still only a median miss.
        let mut runs = recorded(RUN_A);
        for run in runs
            .iter_mut()
            .filter(|run| matches!(run.case, Case::NewProject | Case::ColdExisting))
        {
            run.ready_ms = run.ready_ms.map(|ready| ready + 400.0);
        }
        assert_eq!(Decision::decide(&verdict(&runs), None), Decision::Remeasure);
    }

    #[test]
    fn a_median_miss_requests_a_second_series_which_passes_the_gate() {
        let first = median_miss(RUN_A);
        let decision = Decision::decide(&first, None);
        assert_eq!(decision, Decision::Remeasure);
        assert!(!decision.passed());
        assert_eq!(decision.series(), None);
        assert!(
            decision
                .render()
                .contains("a second series decides: the first series held every run")
        );

        let second = passing();
        let decision = Decision::decide(&first, Some(&second));
        assert_eq!(decision, Decision::PassedOnSecond);
        assert!(decision.passed());
        assert_eq!(decision.series(), Some(2));
        assert!(
            decision
                .render()
                .contains("passed, decided by the second series")
        );
    }

    #[test]
    fn a_second_median_miss_fails_the_gate() {
        let first = median_miss(RUN_A);
        let decision = Decision::decide(&first, Some(&median_miss(RUN_B)));
        assert_eq!(decision, Decision::FailedOnSecond);
        assert!(!decision.passed());
        assert_eq!(decision.series(), Some(2));
        assert!(
            decision
                .render()
                .contains("failed, decided by the second series")
        );
    }

    #[test]
    fn a_count_violation_or_failed_open_in_the_second_series_fails_the_gate() {
        let first = median_miss(RUN_A);
        for second in [count_violation(RUN_B), failed_open(RUN_B)] {
            let decision = Decision::decide(&first, Some(&second));
            assert_eq!(decision, Decision::FailedOnSecond, "{second:?}");
            assert!(!decision.passed());
        }
    }

    #[test]
    fn a_failed_judgement_names_the_deciding_series_and_both_series_violations() {
        let first = median_miss(RUN_A);
        let second = count_violation(RUN_B);
        let judgement = Judgement {
            decision: Decision::decide(&first, Some(&second)),
            first,
            second: Some(second),
        };
        assert!(!judgement.passed());
        let text = judgement.describe(&ci_gate());
        let (second, first) = text
            .split_once("\nfirst series:\n")
            .unwrap_or_else(|| panic!("{text}"));
        assert!(
            second.starts_with("second series (decides):\n")
                && second.contains("new-project: 4 engine starts, expected 2, in iteration 3"),
            "{text}"
        );
        // Run A's new-project median 729.75 ms, plus 200.
        assert!(
            first.contains("new-project: median open to ready 929.8 ms is over the 890 ms budget"),
            "{text}"
        );

        let judgement = Judgement {
            decision: Decision::decide(&failed_open(RUN_A), None),
            first: failed_open(RUN_A),
            second: None,
        };
        assert_eq!(
            judgement.describe(&ci_gate()),
            "cold-existing: no readiness signal (a failed open), in iteration 2"
        );
    }

    #[test]
    fn a_series_section_carries_its_own_title() {
        let text = render_titled(
            &median_miss(RUN_A),
            &ci_gate(),
            "Open-time gate, first series",
        );
        assert!(
            text.contains("## Open-time gate, first series: failed"),
            "{text}"
        );
        assert!(!text.contains("## Open-time gate: "), "{text}");
    }
}
