//! Test-only wait for a fixture operation that ends on the event it waits for.
//!
//! A flat `timeout(N, poll)` decides an outcome on a number guessed at runner
//! speed: a loaded runner that is still making progress fails it. This wait
//! instead ends when its condition holds or the watched task finishes, and
//! fails only when the operation stops making observable progress for one
//! whole gap bound, which a fixture derives from the stated budgets of the
//! steps between its progress signals ([`dream_gap_bound`]).

use std::{sync::Arc, time::Duration};

use kuru_connectors::HookHost;
use kuru_core::{HookCommand, HookEvent};
use tokio::{
    sync::broadcast,
    task::{JoinError, JoinHandle},
    time::Instant,
};

use crate::{Event, step_timings::StepTimings};

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

/// The longest silence a fixture without lifecycle hooks accepts between
/// progress signals while its task settles: one memory operation's stated
/// budget, [`crate::tests::turn_admission_deadline`], the memory startup
/// budget from which the Dolt listener derives its statement read timeout.
///
/// That statement bound equals this gap rather than lying below it, as for the
/// memory term of [`dream_gap_bound`]: a statement that exhausts it fails the
/// fixture either way, and only the diagnostic differs. The gap is strictly
/// greater than the hook host's quiesce bound, so an enclosed quiesce with no
/// worker in flight stays inside it. Fails when the host has hooks
/// configured; such a fixture derives its bound with [`dream_gap_bound`].
pub(crate) fn unhooked_gap_bound(hooks: &HookHost) -> Duration {
    let configured = [
        HookEvent::PreTurn,
        HookEvent::PostTurn,
        HookEvent::PreTool,
        HookEvent::PostTool,
        HookEvent::SpeakerSelected,
    ]
    .into_iter()
    .filter(|event| hooks.configured(*event))
    .map(HookEvent::label)
    .collect::<Vec<_>>();
    assert!(
        configured.is_empty(),
        "fixture configures {configured:?} hooks; derive its gap with dream_gap_bound"
    );
    crate::tests::turn_admission_deadline()
}

/// What a fixture can observe of a task that owns its moved harness: step
/// marks, harness events and the hook host's in-flight workers are its
/// progress; the rendered steps and events are failure context.
pub(crate) struct TaskWatch {
    pub(crate) timings: StepTimings,
    events: broadcast::Receiver<Event>,
    seen: Vec<String>,
    pub(crate) hooks: Arc<HookHost>,
}

impl TaskWatch {
    pub(crate) fn new(
        timings: StepTimings,
        events: broadcast::Receiver<Event>,
        hooks: Arc<HookHost>,
    ) -> Self {
        Self {
            timings,
            events,
            seen: vec![],
            hooks,
        }
    }

    /// Install a recording step timer on `harness` before it moves into its
    /// task, and watch its events and hook workers.
    pub(crate) fn attach(harness: &mut crate::Harness) -> Self {
        let timings = StepTimings::recording();
        harness.step_timings = timings.clone();
        Self::new(timings, harness.subscribe(), harness.hook_host())
    }

    pub(crate) fn progress(&mut self) -> (usize, usize, usize) {
        use tokio::sync::broadcast::error::TryRecvError;
        loop {
            match self.events.try_recv() {
                Ok(event) => self
                    .seen
                    .push(format!("{event:?}").chars().take(240).collect()),
                Err(TryRecvError::Lagged(skipped)) => {
                    self.seen.push(format!("({skipped} events skipped)"));
                }
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        (
            self.timings.completed(),
            self.seen.len(),
            self.hooks.in_flight_hooks(),
        )
    }

    /// Step timings, in-flight hook workers and every observed event.
    pub(crate) fn report(&mut self) -> String {
        self.progress();
        let events = if self.seen.is_empty() {
            "(none)".into()
        } else {
            self.seen.join("\n  ")
        };
        format!(
            "steps (time since the test created its recorder):\n{}\n\
             in-flight hook workers: {}\nevents observed:\n  {events}",
            self.timings.render(),
            self.hooks.in_flight_hooks(),
        )
    }
}

/// `Ok` or the error and whether it was a cancellation, for a joined task's
/// failure report.
pub(crate) fn describe_result<T>(result: &anyhow::Result<T>) -> String {
    match result {
        Ok(_) => "Ok".into(),
        Err(error) => format!(
            "Err({error:#}); cancelled: {}",
            crate::turn_was_cancelled(error)
        ),
    }
}

/// Join a task the fixture no longer drives (after cancelling it, or after
/// releasing its last pause) on the event of it finishing. Every observed
/// progress signal re-arms `gap`; panics with the [`try_join_on_progress`]
/// report when the task panics or stays silent for one whole gap.
pub(crate) async fn join_on_progress<T>(
    task: &mut JoinHandle<T>,
    watch: &mut TaskWatch,
    gap: Duration,
    what: &str,
    show: impl FnOnce(T) -> String,
) -> T {
    match try_join_on_progress(task, watch, gap, what, show).await {
        Ok(output) => output,
        Err(report) => panic!("{report}"),
    }
}

/// [`join_on_progress`] without the panic. On a stall the report is read
/// before anything else happens, names the last completed step, and then the
/// task is awaited once more under the same gap and aborted if it still has
/// not finished, so the report separates a slow finish from a hang.
pub(crate) async fn try_join_on_progress<T>(
    task: &mut JoinHandle<T>,
    watch: &mut TaskWatch,
    gap: Duration,
    what: &str,
    show: impl FnOnce(T) -> String,
) -> Result<T, String> {
    match until_event(task, || false, || watch.progress(), gap).await {
        Waited::Finished(Ok(output)) => Ok(output),
        Waited::Finished(Err(error)) => Err(format!(
            "the {what} task panicked or was aborted: {error}\n{}",
            watch.report()
        )),
        Waited::Reached => unreachable!("a join waits for no condition"),
        Waited::Stalled { progress_changes } => {
            let last = watch.timings.last();
            let report = watch.report();
            Err(format!(
                "the {what} made no observable progress for {gap:?} while settling \
                 ({progress_changes} progress changes seen during the join); \
                 last completed step: {last}\n{report}\nafter the stall, {}",
                settle(task, gap, show).await
            ))
        }
    }
}

/// Await a task that has already been reported once more under `gap`,
/// aborting it if it still does not finish.
pub(crate) async fn settle<T>(
    task: &mut JoinHandle<T>,
    gap: Duration,
    show: impl FnOnce(T) -> String,
) -> String {
    match tokio::time::timeout(gap, &mut *task).await {
        Ok(Ok(output)) => format!("it finished: {}", show(output)),
        Ok(Err(error)) => format!("it panicked or was aborted: {error}"),
        Err(_) => {
            task.abort();
            format!("it did not finish within a further {gap:?}; it was aborted")
        }
    }
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
        kuru_memory::test_support::closing(async {
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
            let waited =
                until_event(&mut task, exists(&marker), || timings.completed(), gap()).await;
            assert!(matches!(waited, Waited::Reached));
            assert!(started.elapsed() >= interval * 3);
            assert_eq!(timings.completed(), 3);
            task.abort();
        })
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn silent_operation_stalls_after_one_gap_and_names_its_last_step() {
        kuru_memory::test_support::closing(async {
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
            let waited =
                until_event(&mut task, exists(&marker), || timings.completed(), gap()).await;
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
        })
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn operation_that_finishes_first_returns_its_outcome_without_waiting_a_bound() {
        kuru_memory::test_support::closing(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("never");
            let mut task = tokio::spawn(async { 7 });
            let started = Instant::now();
            let waited = until_event(&mut task, exists(&marker), || 0, gap()).await;
            assert!(matches!(waited, Waited::Finished(Ok(7))));
            assert!(started.elapsed() < gap());
        })
        .await
    }

    fn hook_host(root: &Path, hooks: kuru_core::LifecycleHooks) -> Arc<HookHost> {
        use kuru_platform::fs::{Directory, NameRetention, Privacy};
        let directory =
            Arc::new(Directory::open(root, Privacy::Inherited, NameRetention::Pinned).unwrap());
        Arc::new(HookHost::new(directory, hooks))
    }

    /// A watch with no harness: its own recorder, an event channel whose
    /// sender the caller keeps, and a hook host with nothing configured.
    fn detached_watch(root: &Path) -> (TaskWatch, broadcast::Sender<Event>) {
        let (events, receiver) = broadcast::channel(16);
        let hooks = hook_host(root, kuru_core::LifecycleHooks::default());
        (
            TaskWatch::new(StepTimings::recording(), receiver, hooks),
            events,
        )
    }

    /// A cancelled task's teardown: mark `steps` steps `interval` apart, then
    /// finish with `7`.
    fn tearing_down(timings: StepTimings, steps: usize, interval: Duration) -> JoinHandle<u8> {
        tokio::spawn(async move {
            for step in 0..steps {
                tokio::time::sleep(interval).await;
                timings.mark(format!("teardown step {step}"));
            }
            7
        })
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_teardown_progressing_beyond_a_flat_ten_seconds_is_joined() {
        kuru_memory::test_support::closing(async {
            let directory = tempfile::tempdir().unwrap();
            let (mut watch, _events) = detached_watch(directory.path());
            let gap = unhooked_gap_bound(&watch.hooks);
            let interval = gap * 2 / 3;
            assert!(interval * 3 > Duration::from_secs(10));
            // The old shape: one flat 10 s join fails a teardown that never
            // stops progressing.
            let mut old = tearing_down(StepTimings::recording(), 3, interval);
            assert!(
                tokio::time::timeout(Duration::from_secs(10), &mut old)
                    .await
                    .is_err()
            );
            old.abort();
            // The progress-aware join ends on the task finishing.
            let started = Instant::now();
            let mut task = tearing_down(watch.timings.clone(), 3, interval);
            let joined =
                try_join_on_progress(&mut task, &mut watch, gap, "teardown", |n| n.to_string())
                    .await;
            assert_eq!(joined, Ok(7));
            assert!(started.elapsed() >= interval * 3);
            assert_eq!(watch.timings.completed(), 3);
        })
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn silent_teardown_is_reported_with_its_last_step_and_aborted() {
        kuru_memory::test_support::closing(async {
            let directory = tempfile::tempdir().unwrap();
            let (mut watch, events) = detached_watch(directory.path());
            let gap = unhooked_gap_bound(&watch.hooks);
            let mut task = tokio::spawn({
                let timings = watch.timings.clone();
                async move {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    timings.mark("teardown step before the silence");
                    let _ = events.send(Event::Error {
                        actor: "pool".into(),
                        detail: "observed teardown event".into(),
                    });
                    std::future::pending::<u8>().await
                }
            });
            let started = Instant::now();
            let report =
                try_join_on_progress(&mut task, &mut watch, gap, "teardown", |n| n.to_string())
                    .await
                    .unwrap_err();
            assert!(
                report.contains(&format!("no observable progress for {gap:?}")),
                "{report}"
            );
            // One step mark and one event, observed in the same poll.
            assert!(report.contains("(1 progress changes seen"), "{report}");
            assert!(
                report.contains("last completed step: teardown step before the silence at +"),
                "{report}"
            );
            assert!(report.contains("in-flight hook workers: 0"), "{report}");
            assert!(report.contains("observed teardown event"), "{report}");
            assert!(report.contains("it was aborted"), "{report}");
            // Silence is measured from the last progress, then the task gets one
            // more gap before it is aborted.
            assert!(started.elapsed() >= Duration::from_secs(1) + gap * 2);
            assert!((&mut task).await.unwrap_err().is_cancelled());
        })
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn panicking_teardown_is_reported_without_waiting_a_gap() {
        kuru_memory::test_support::closing(async {
            let directory = tempfile::tempdir().unwrap();
            let (mut watch, _events) = detached_watch(directory.path());
            let gap = unhooked_gap_bound(&watch.hooks);
            let mut task = tokio::spawn(async { panic!("teardown failed") });
            let started = Instant::now();
            let report =
                try_join_on_progress(&mut task, &mut watch, gap, "teardown", |()| String::new())
                    .await
                    .unwrap_err();
            assert!(
                report.contains("the teardown task panicked or was aborted"),
                "{report}"
            );
            assert!(started.elapsed() < gap);
        })
        .await
    }

    #[test]
    fn unhooked_gap_bound_is_the_memory_budget_strictly_above_quiesce() {
        let directory = tempfile::tempdir().unwrap();
        let hooks = hook_host(directory.path(), kuru_core::LifecycleHooks::default());
        let gap = unhooked_gap_bound(&hooks);
        assert_eq!(gap, crate::tests::turn_admission_deadline());
        assert!(gap > hooks.quiesce_bound());
    }

    #[test]
    #[should_panic(expected = "derive its gap with dream_gap_bound")]
    fn unhooked_gap_bound_refuses_a_fixture_with_hooks() {
        let directory = tempfile::tempdir().unwrap();
        let hooks = kuru_core::LifecycleHooks {
            post_tool: vec![HookCommand {
                command: "hook".into(),
                args: vec![],
                timeout_ms: 1,
                max_output_bytes: 1,
            }],
            ..kuru_core::LifecycleHooks::default()
        };
        unhooked_gap_bound(&hook_host(directory.path(), hooks));
    }

    #[test]
    fn describe_result_names_a_cancellation() {
        assert_eq!(describe_result(&anyhow::Ok(())), "Ok");
        let failed = describe_result::<()>(&Err(anyhow::anyhow!("plain failure")));
        assert_eq!(failed, "Err(plain failure); cancelled: false");
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
