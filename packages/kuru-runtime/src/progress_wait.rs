//! Test-only wait for a fixture operation that ends on the event it waits for.
//!
//! A flat `timeout(N, poll)` decides an outcome on a number guessed at runner
//! speed: a loaded runner that is still making progress fails it. This wait
//! instead ends when its condition holds or the watched task finishes, and
//! fails only when the operation stops making observable progress for one
//! whole gap bound, which a fixture derives from the stated budgets of the
//! steps between its progress signals ([`dream_gap_bound`]).

use std::time::Duration;

use kuru_core::HookCommand;
use tokio::{
    task::{JoinError, JoinHandle},
    time::Instant,
};

/// How often the condition and progress are re-read. Cadence only: it never
/// decides an outcome.
const POLL: Duration = Duration::from_millis(10);

/// How a [`until_event`] wait ended.
pub(crate) enum Waited<T> {
    /// The condition held while the task was still running.
    Reached,
    /// The task finished before the condition held.
    Finished(Result<T, JoinError>),
    /// No progress signal changed for one whole gap bound.
    Stalled { progress_changes: usize },
}

/// Wait until `reached` holds or `task` finishes. Every change of the
/// `progress` value re-arms the gap bound; only `gap` of silence is a stall.
/// The task is left running on `Reached` and `Stalled`, so the caller can
/// cancel it and report what it was doing.
pub(crate) async fn until_event<T, P: PartialEq>(
    task: &mut JoinHandle<T>,
    mut reached: impl FnMut() -> bool,
    mut progress: impl FnMut() -> P,
    gap: Duration,
) -> Waited<T> {
    let mut last = progress();
    let mut changes = 0;
    let mut silent_since = Instant::now();
    loop {
        if reached() {
            return Waited::Reached;
        }
        if task.is_finished() {
            return Waited::Finished(task.await);
        }
        let now = progress();
        if now != last {
            last = now;
            changes += 1;
            silent_since = Instant::now();
        } else if silent_since.elapsed() >= gap {
            return Waited::Stalled {
                progress_changes: changes,
            };
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The longest silence a dream fixture accepts between progress signals.
///
/// Step-timing marks run densely through the dream and actor path, and
/// harness events plus the in-flight hook count cover the tool, hook and
/// annotation steps, so one silent gap holds one product-bounded step: a
/// memory operation or one lifecycle hook run. A gap may hold a few short
/// memory statements; the bound is the larger single stated budget, not
/// their sum:
/// - memory: [`crate::tests::turn_admission_deadline`], the memory startup
///   budget from which the Dolt listener derives its statement read timeout;
/// - hook: the hook's configured `timeout_ms` plus the host's quiesce bound,
///   which covers owned tree cleanup after the timeout and is the whole
///   product wait after a cancellation (`Harness::await_hook_cleanup`).
///
/// With a nonzero hook timeout the result is strictly greater than the
/// quiesce bound a cancelled dream's join encloses.
pub(crate) fn dream_gap_bound(hook: &HookCommand, quiesce: Duration) -> Duration {
    let memory = crate::tests::turn_admission_deadline();
    let hook = Duration::from_millis(hook.timeout_ms).saturating_add(quiesce);
    memory.max(hook)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::step_timings::StepTimings;

    /// A synthetic operation: mark `steps` progress steps `interval` apart,
    /// then write `marker` and stay running until aborted.
    fn progressing(
        timings: StepTimings,
        steps: usize,
        interval: Duration,
        marker: PathBuf,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            for step in 0..steps {
                tokio::time::sleep(interval).await;
                timings.mark(format!("step {step}"));
            }
            std::fs::write(&marker, "started").unwrap();
            std::future::pending::<()>().await;
        })
    }

    fn gap() -> Duration {
        crate::tests::turn_admission_deadline()
    }

    fn exists(marker: &Path) -> impl FnMut() -> bool + '_ {
        move || marker.exists()
    }

    /// The old shape: one flat 10 s deadline for the whole operation.
    async fn flat_ten_seconds(marker: &Path) -> bool {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !marker.exists() {
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .is_ok()
    }

    #[tokio::test(start_paused = true)]
    async fn progressing_operation_beyond_a_flat_ten_seconds_reaches_its_event() {
        let interval = gap() * 2 / 3;
        assert!(interval * 3 > Duration::from_secs(10));
        // The old flat wait fails an operation that never stops progressing.
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("old-shape");
        let mut old = progressing(StepTimings::recording(), 3, interval, marker.clone());
        assert!(!flat_ten_seconds(&marker).await);
        old.abort();
        assert!((&mut old).await.unwrap_err().is_cancelled());
        // The progress-aware wait accepts it: no silent gap reaches the bound.
        let marker = directory.path().join("new-shape");
        let timings = StepTimings::recording();
        let started = Instant::now();
        let mut task = progressing(timings.clone(), 3, interval, marker.clone());
        let waited = until_event(&mut task, exists(&marker), || timings.completed(), gap()).await;
        assert!(matches!(waited, Waited::Reached));
        assert!(started.elapsed() >= interval * 3);
        assert_eq!(timings.completed(), 3);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn silent_operation_stalls_after_one_gap_and_names_its_last_step() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("never");
        let timings = StepTimings::recording();
        let mut task = tokio::spawn({
            let timings = timings.clone();
            async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                timings.mark("last completed step");
                std::future::pending::<()>().await
            }
        });
        let started = Instant::now();
        let waited = until_event(&mut task, exists(&marker), || timings.completed(), gap()).await;
        assert!(matches!(
            waited,
            Waited::Stalled {
                progress_changes: 1
            }
        ));
        // Silence is measured from the last progress, not from the start.
        assert!(started.elapsed() >= Duration::from_secs(1) + gap());
        assert!(timings.render().contains("last completed step"));
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn operation_that_finishes_first_returns_its_outcome_without_waiting_a_bound() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("never");
        let mut task = tokio::spawn(async { 7 });
        let started = Instant::now();
        let waited = until_event(&mut task, exists(&marker), || 0, gap()).await;
        assert!(matches!(waited, Waited::Finished(Ok(7))));
        assert!(started.elapsed() < gap());
    }

    #[test]
    fn dream_gap_bound_takes_the_larger_stated_budget_above_quiesce() {
        let hook = |timeout_ms| HookCommand {
            command: "hook".into(),
            args: vec![],
            timeout_ms,
            max_output_bytes: 1,
        };
        let quiesce = Duration::from_secs(10);
        let memory = crate::tests::turn_admission_deadline();
        assert_eq!(
            dream_gap_bound(&hook(1), quiesce),
            memory.max(quiesce + Duration::from_millis(1))
        );
        let long = memory.as_millis() as u64 + 1;
        assert_eq!(
            dream_gap_bound(&hook(long), quiesce),
            Duration::from_millis(long) + quiesce
        );
        assert!(dream_gap_bound(&hook(1), quiesce) > quiesce);
    }
}
