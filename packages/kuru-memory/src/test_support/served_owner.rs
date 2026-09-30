//! In-process service owners that a fixture serves beneath its guarded root.
//!
//! A fixture that runs [`ServiceOwner::serve`] on a task must retire that
//! owner before its root drops, or the root's guard ([`super::TempDir`])
//! fails the test. When the retirement ran only at the end of the block that
//! also owned the root, an early `?`, `bail!` or `ensure!` detached the
//! served task and dropped the root under a live engine; and when the root
//! and the owner lived inside the future its fixture deadline drops, an
//! elapsed deadline did the same. The guard then correctly failed the test,
//! but its panic replaced the fixture's own error.
//!
//! So such a fixture keeps its root and its options outside its
//! [`FixtureDeadline`], and serves its owner through
//! [`FixtureDeadline::serve`], which holds the [`ServedOwner`] outside the
//! timed future. The fixture's stage opens and serves the owner and returns
//! the body's `Result`; its teardown is the success tail's own retirement.
//! Within the deadline, [`settle`] writes a body error to the test's captured
//! output before the teardown starts, then runs the teardown with the same
//! calls and bounds as the success path. When the deadline elapses, the timed
//! stage is dropped (its clients with it), the deadline's own error is
//! written first, and then the same teardown runs, bounded by its own calls
//! (the maintenance permit by the startup budget, the reap by its bound). The
//! returned error is the body's or the deadline's, with any teardown failure
//! attached. The fixture then releases its root with [`super::TempDir::release`],
//! which attaches the guard's verdict to that error when the teardown could
//! not retire the owner (a client task aborted mid-request can legitimately
//! keep the owner busy): on every exit path the owner is retired or the
//! verdict is reported, and neither replaces the fixture's error.
//!
//! [`ServedOwner::serve`] serves the in-process fixture policy: never reached
//! by a starter, so the owner outlives its last client until the fixture
//! retires it. A test of the serve loop itself chooses its policy with
//! [`ServedOwner::serve_with`] and awaits the loop's ordered events through
//! [`observed`], [`next_event`] and [`expect_events`], never through a sleep.

use crate::{
    OpenOptions,
    service::{Admission, ServeEvent, ServeKnobs, ServiceOwner, acquire_maintenance_permit},
};
use anyhow::{Context, Error, Result, anyhow, bail, ensure};
use std::{future::Future, path::Path, time::Duration};
use tokio::{
    sync::{RwLockReadGuard, mpsc},
    task::JoinHandle,
    time::Instant,
};

/// The owner task a fixture is serving, or nothing once that task has ended.
///
/// A finished task is never polled again: [`ServedOwner::reap`] empties the
/// slot as soon as the task has ended, whatever the owner returned.
pub(crate) struct ServedOwner {
    task: Option<JoinHandle<Result<()>>>,
}

impl ServedOwner {
    /// Serve `owner` on a new task under the in-process fixture policy,
    /// [`default_knobs`]: the fixture's first owner, or a successor after the
    /// previous owner has been reaped.
    pub(crate) fn serve(&mut self, owner: ServiceOwner) -> Result<()> {
        self.serve_with(owner, default_knobs())
    }

    /// [`Self::serve`] under the fixture's own policy: who reaches the owner,
    /// how long it waits for its starter, and the serve-loop observer and
    /// pauses a test asserts through. Pair a chosen policy with
    /// [`observed`] to await the loop's events instead of sleeping.
    pub(crate) fn serve_with(&mut self, owner: ServiceOwner, knobs: ServeKnobs) -> Result<()> {
        ensure!(
            self.task.is_none(),
            "a fixture owner is still being served; reap it before its successor"
        );
        self.task = Some(tokio::spawn(owner.serve_with(knobs)));
        Ok(())
    }

    /// Whether the served task has ended and has not been reaped yet.
    pub(crate) fn is_finished(&self) -> bool {
        self.task.as_ref().is_some_and(JoinHandle::is_finished)
    }

    /// Await the owner's exit within `within`, and return what it returned.
    /// After a timeout the task stays in the slot for the fixture's teardown.
    pub(crate) async fn reap(&mut self, within: Duration, context: &str) -> Result<()> {
        let task = self
            .task
            .as_mut()
            .with_context(|| format!("{context}: no fixture owner is being served"))?;
        let joined = tokio::time::timeout(within, task)
            .await
            .with_context(|| context.to_owned())?;
        self.task = None;
        joined?
    }

    /// Retire the served owner with a fixture's success-tail calls: acquire
    /// the maintenance permit (within `permit_within` when the fixture bounds
    /// it), await the owner's reap within `reap_within`, then release the
    /// permit. With no owner being served there is nothing to retire. An owner
    /// that already retired itself under [`Self::serve_with`] is fine: the
    /// permit then waits out its unlock, and the reap joins the finished task.
    pub(crate) async fn retire(
        &mut self,
        options: &OpenOptions,
        permit_within: Option<Duration>,
        reap_within: Duration,
        context: &str,
    ) -> Result<()> {
        if self.task.is_none() {
            return Ok(());
        }
        let permit = match permit_within {
            None => acquire_maintenance_permit(options).await,
            Some(bound) => tokio::time::timeout(bound, acquire_maintenance_permit(options))
                .await
                .with_context(|| format!("{context}: the owner did not retire within {bound:?}"))?,
        }
        .with_context(|| format!("{context}: the owner was not retired"))?;
        self.reap(reap_within, context).await?;
        drop(permit);
        Ok(())
    }

    /// Retire the served owner as [`Self::retire`] does, then open and serve
    /// its successor, all with every other in-binary spawn excluded. Takes the
    /// caller's shared spawn guard and returns a new one.
    ///
    /// [`ServiceOwner::open`] takes the owner lock once and never waits, as it
    /// does in the product, where only a freshly elected owner process calls
    /// it. In this multi-threaded test binary a sibling test's child can hold a
    /// duplicate of the retired owner's lock description between its spawn and
    /// its exec, so the successor would see a busy lock although no owner
    /// exists; see [`crate::spawn_gate::excluding_spawns`].
    pub(crate) async fn restart(
        &mut self,
        gate: RwLockReadGuard<'static, ()>,
        options: &OpenOptions,
        project: &Path,
        reap_within: Duration,
        context: &str,
    ) -> Result<RwLockReadGuard<'static, ()>> {
        let ((), gate) = crate::spawn_gate::excluding_spawns(gate, async {
            self.retire(options, None, reap_within, context).await?;
            let successor = ServiceOwner::open(options.clone(), project)
                .await
                .with_context(|| format!("{context}: its successor did not open"))?;
            self.serve(successor)
        })
        .await?;
        Ok(gate)
    }
}

/// The policy [`ServedOwner::serve`] serves: never reached by a starter, and
/// no first-attachment deadline. An in-process fixture owner then ends only
/// through maintenance retirement, [`ServedOwner::restart`] or lock loss, not
/// at its last detach. Retiring there would leave a later attach to spawn the
/// libtest binary as the successor, which cannot serve; the product policy
/// (retire at the last detach once the starter has attached) is covered by
/// really spawned owners and by tests that choose it through
/// [`ServedOwner::serve_with`].
fn default_knobs() -> ServeKnobs {
    ServeKnobs::never_reached()
}

/// The ordered serve-loop events of an [`observed`] owner.
pub(crate) type ServeEvents = mpsc::UnboundedReceiver<ServeEvent>;

/// Serve-loop knobs that report to the returned ordered event log. The log is
/// unbounded and its events are sent before the loop acts on them, so a test
/// that awaits them in order proves the sequence without sleeping. Start from
/// the returned knobs to add a close or dispatch pause.
pub(crate) fn observed(
    admission: Admission,
    first_attachment: Option<Duration>,
) -> (ServeKnobs, ServeEvents) {
    let (observer, events) = mpsc::unbounded_channel();
    let knobs = ServeKnobs {
        admission,
        first_attachment,
        observer: Some(observer),
        ..ServeKnobs::never_reached()
    };
    (knobs, events)
}

/// The next event other than a lock recheck, which recurs on the owner's own
/// schedule. A closed log means the serve loop has ended.
pub(crate) async fn next_event(events: &mut ServeEvents) -> Result<ServeEvent> {
    loop {
        match events
            .recv()
            .await
            .context("the serve loop ended before the expected event")?
        {
            ServeEvent::LockRechecked => {}
            event => return Ok(event),
        }
    }
}

/// Await each of `expected`, in order, as the next non-recheck event.
pub(crate) async fn expect_events(events: &mut ServeEvents, expected: &[ServeEvent]) -> Result<()> {
    for expected in expected {
        let event = next_event(events).await?;
        ensure!(
            event == *expected,
            "serve event {event:?}, expected {expected:?}"
        );
    }
    Ok(())
}

/// No event other than a lock recheck has been sent since the last one
/// awaited. Every owner event is sent before the loop acts on it, so an empty
/// log after a completed client exchange is a settled one.
pub(crate) fn expect_no_event(events: &mut ServeEvents, context: &str) -> Result<()> {
    loop {
        match events.try_recv() {
            Ok(ServeEvent::LockRechecked) => {}
            Ok(event) => bail!("{context}: unexpected serve event {event:?}"),
            Err(mpsc::error::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                bail!("{context}: the serve loop ended")
            }
        }
    }
}

/// A fixture's outer hang backstop: one absolute instant, started once and
/// shared by every stage and iteration of the fixture, so a stage that runs
/// outside [`Self::serve`] or a later iteration never gets a fresh budget.
pub(crate) struct FixtureDeadline {
    budget: Duration,
    at: Instant,
    fixture: &'static str,
}

impl FixtureDeadline {
    /// Start `budget` now for the fixture named `fixture` in its error.
    pub(crate) fn start(budget: Duration, fixture: &'static str) -> Self {
        Self {
            budget,
            at: Instant::now() + budget,
            fixture,
        }
    }

    /// The error an elapsed deadline reports.
    fn elapsed(&self) -> Error {
        anyhow!("{} exceeded its {:?} deadline", self.fixture, self.budget)
    }

    /// Run a stage that serves no owner before the deadline.
    pub(crate) async fn run<T>(&self, stage: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::time::timeout_at(self.at, stage)
            .await
            .unwrap_or_else(|_| Err(self.elapsed()))
    }

    /// Run `stage`, which serves its owner through the [`ServedOwner`] it is
    /// given, and then `teardown` on every exit path: after the stage
    /// returns, within the deadline, as [`settle`] does, and after the
    /// deadline elapses, once the stage has been dropped. The owner lives
    /// here, outside the timed future, so an elapsed deadline cannot detach
    /// it before its teardown.
    pub(crate) async fn serve<T>(
        &self,
        stage: impl AsyncFnOnce(&mut ServedOwner) -> Result<T>,
        teardown: impl AsyncFn(&mut ServedOwner) -> Result<()>,
    ) -> Result<T> {
        serve_until(
            tokio::time::sleep_until(self.at),
            || self.elapsed(),
            stage,
            teardown,
        )
        .await
    }
}

/// [`FixtureDeadline::serve`] for a fixture that has no outer deadline: the
/// same stage and teardown, with the teardown run after the stage returns.
pub(crate) async fn serve_without_deadline<T>(
    stage: impl AsyncFnOnce(&mut ServedOwner) -> Result<T>,
    teardown: impl AsyncFn(&mut ServedOwner) -> Result<()>,
) -> Result<T> {
    serve_until(
        std::future::pending(),
        || anyhow!("a fixture without a deadline cannot expire"),
        stage,
        teardown,
    )
    .await
}

/// [`FixtureDeadline::serve`] with the deadline as any future, so a test can
/// expire it at a chosen point of the stage.
async fn serve_until<T>(
    expired: impl Future<Output = ()>,
    elapsed: impl FnOnce() -> Error,
    stage: impl AsyncFnOnce(&mut ServedOwner) -> Result<T>,
    teardown: impl AsyncFn(&mut ServedOwner) -> Result<()>,
) -> Result<T> {
    let mut served = ServedOwner { task: None };
    let finished = {
        let timed = async {
            let body = stage(&mut served).await;
            settle(body, teardown(&mut served)).await
        };
        tokio::select! {
            biased;
            result = timed => Some(result),
            () = expired => None,
        }
    };
    if let Some(result) = finished {
        return result;
    }
    let error = elapsed();
    eprintln!("{error:#}; retiring its service owner before the root is released");
    let teardown = teardown(&mut served).await;
    combine(Err(error), teardown)
}

/// Finish a fixture whose `body` ran while its root and served owner lived:
/// write a body error to the test's captured output, then run `teardown` on
/// every path. The body's error is returned in preference to the teardown's;
/// when both fail, the teardown's error is attached to it as context.
pub(crate) async fn settle<T>(
    body: Result<T>,
    teardown: impl Future<Output = Result<()>>,
) -> Result<T> {
    if let Err(error) = &body {
        eprintln!(
            "fixture body failed; retiring its service owner before the root is released: \
             {error:?}"
        );
    }
    combine(body, teardown.await)
}

fn combine<T>(body: Result<T>, teardown: Result<()>) -> Result<T> {
    match (body, teardown) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(teardown)) => Err(teardown),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(teardown)) => Err(error.context(format!(
            "the fixture body failed, and its owner teardown then failed too: {teardown:#}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    const REAP: Duration = Duration::from_secs(10);

    /// A guarded root, a canonical project inside it and its options.
    fn fixture() -> Result<(super::super::TempDir, PathBuf, OpenOptions)> {
        let root = super::super::tempdir()?;
        let project = root.path().join("project");
        std::fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
        let scope = format!(
            "project/{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let options = super::super::open_options(root.path().join("private"), scope)?;
        Ok((root, project, options))
    }

    /// A stopped store with no quiescence record: the guard's verdict for it
    /// is a violation, and no engine ever ran.
    fn unrecorded_store(root: &Path) -> Result<PathBuf> {
        let store = root.join("memory").join("abcdef");
        for directory in [root.join("memory"), store.clone()] {
            crate::files::private_dir(&directory)?;
        }
        crate::files::write(&store.join("server.log"), b"Kuru engine shutdown: Ok")?;
        crate::files::write(&store.join("lifecycle.lock"), b"")?;
        Ok(store)
    }

    /// Attach a bare client to the fixture's published owner.
    async fn attach(
        options: &OpenOptions,
        project: &Path,
    ) -> Result<crate::service::ServiceAttachment> {
        crate::service::attach_or_start(options, project, &std::env::current_exe()?).await
    }

    /// `serve_with` serves the policy the fixture chose. Reached by any
    /// attachment, the owner retires itself at the last detach: every event
    /// up to `EnteredEmpty { reached: true }` is in order, the serve task then
    /// ends cleanly, and its owner lock and endpoint are gone.
    #[tokio::test]
    async fn serve_with_applies_the_fixture_policy_and_retires_at_the_last_detach() -> Result<()> {
        super::super::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let budget = super::super::fixture_deadline(1, 0);
        let deadline = FixtureDeadline::start(budget, "serve_with fixture");
        let (root, project, options) = fixture()?;
        let (knobs, mut events) = observed(Admission::AnyAttachment, None);
        let settled = deadline
            .serve(
                async |served| {
                    let _gate = crate::spawn_gate::spawning().await;
                    let owner = ServiceOwner::open(options.clone(), &project).await?;
                    served.serve_with(owner, knobs)?;
                    expect_events(&mut events, &[ServeEvent::EnteredEmpty { reached: false }])
                        .await?;
                    let attachment = attach(&options, &project).await?;
                    expect_events(&mut events, &[ServeEvent::AttachmentAccepted { active: 1 }])
                        .await?;
                    drop(attachment);
                    expect_events(
                        &mut events,
                        &[
                            ServeEvent::AttachmentJoined { remaining: 0 },
                            ServeEvent::EnteredEmpty { reached: true },
                        ],
                    )
                    .await?;
                    served
                        .reap(budget, "the reached owner did not retire")
                        .await?;
                    ensure!(
                        crate::service::EndpointRecord::read(
                            &options.data_dir,
                            &options.project_scope
                        )?
                        .is_none(),
                        "the retired owner left its endpoint"
                    );
                    Ok(())
                },
                async |served| {
                    served
                        .retire(
                            &options,
                            None,
                            budget,
                            "serve_with fixture owner did not reap",
                        )
                        .await
                },
            )
            .await;
        root.release(settled)
    }

    /// The default policy for an in-process owner is "never reached": the
    /// last detach only returns the owner to waiting for a starter, in the
    /// ordered log, and the owner keeps serving; maintenance ends it.
    #[tokio::test]
    async fn a_never_reached_owner_outlives_its_last_client_until_maintenance_retires_it()
    -> Result<()> {
        super::super::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let budget = super::super::fixture_deadline(1, 0);
        let deadline = FixtureDeadline::start(budget, "never reached fixture");
        let (root, project, options) = fixture()?;
        let (knobs, mut events) = observed(Admission::Never, None);
        let settled = deadline
            .serve(
                async |served| {
                    let _gate = crate::spawn_gate::spawning().await;
                    let owner = ServiceOwner::open(options.clone(), &project).await?;
                    served.serve_with(owner, knobs)?;
                    expect_events(&mut events, &[ServeEvent::EnteredEmpty { reached: false }])
                        .await?;
                    let first = attach(&options, &project).await?;
                    let generation = first.generation().to_owned();
                    drop(first);
                    expect_events(
                        &mut events,
                        &[
                            ServeEvent::AttachmentAccepted { active: 1 },
                            ServeEvent::AttachmentJoined { remaining: 0 },
                            ServeEvent::EnteredEmpty { reached: false },
                        ],
                    )
                    .await?;
                    ensure!(!served.is_finished(), "the never-reached owner retired");
                    let second = attach(&options, &project).await?;
                    ensure!(
                        second.generation() == generation,
                        "the owner served a new generation after its last client left"
                    );
                    drop(second);
                    expect_events(
                        &mut events,
                        &[
                            ServeEvent::AttachmentAccepted { active: 1 },
                            ServeEvent::AttachmentJoined { remaining: 0 },
                            ServeEvent::EnteredEmpty { reached: false },
                        ],
                    )
                    .await?;
                    expect_no_event(&mut events, "a waiting never-reached owner")?;
                    served
                        .retire(
                            &options,
                            None,
                            budget,
                            "the never-reached owner did not retire",
                        )
                        .await
                },
                async |served| {
                    served
                        .retire(
                            &options,
                            None,
                            budget,
                            "never reached fixture owner did not reap",
                        )
                        .await
                },
            )
            .await;
        root.release(settled)
    }

    /// `serve` is `serve_with` under the never-reached policy: no admission
    /// reaches the owner and no first-attachment deadline expires.
    #[test]
    fn serve_defaults_to_the_never_reached_policy() {
        let knobs = default_knobs();
        assert_eq!(knobs.admission, Admission::Never);
        assert_eq!(knobs.first_attachment, None);
        assert!(knobs.observer.is_none() && knobs.close_pause.is_none());
        assert!(knobs.dispatch_pause.is_none());
    }

    #[tokio::test]
    async fn a_body_error_is_returned_first_with_a_teardown_error_attached() -> Result<()> {
        let both = settle::<()>(Err(anyhow!("body failure")), async {
            Err(anyhow!("teardown failure"))
        })
        .await
        .unwrap_err();
        ensure!(
            both.root_cause().to_string() == "body failure"
                && format!("{both:#}").contains("teardown failure"),
            "a failed body and teardown did not keep the body error with the teardown attached: \
             {both:#}"
        );
        let body = settle::<()>(Err(anyhow!("body failure")), async { Ok(()) })
            .await
            .unwrap_err();
        ensure!(format!("{body:#}") == "body failure");
        let teardown = settle(Ok(7), async { Err(anyhow!("teardown failure")) })
            .await
            .unwrap_err();
        ensure!(format!("{teardown:#}") == "teardown failure");
        ensure!(settle(Ok(7), async { Ok(()) }).await? == 7);
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_body_retires_its_live_owner_before_the_root_is_released() -> Result<()> {
        super::super::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline =
            FixtureDeadline::start(super::super::fixture_deadline(1, 0), "served owner fixture");
        let (root, project, options) = fixture()?;
        let path = root.path().to_path_buf();
        let settled = deadline
            .serve(
                async |served| {
                    let _gate = crate::spawn_gate::spawning().await;
                    let owner = ServiceOwner::open(options.clone(), &project).await?;
                    served.serve(owner)?;
                    let _client = crate::MemoryStore::open_managed_observed(
                        options.clone(),
                        project.clone(),
                        std::env::current_exe()?,
                    )
                    .1
                    .await?;
                    Err::<(), _>(anyhow!(
                        "injected fixture body failure with an attached client"
                    ))
                },
                async |served| {
                    served
                        .retire(&options, None, REAP, "served fixture owner did not reap")
                        .await
                },
            )
            .await;
        // The guard attaches its verdict if the owner's engine outlived it.
        let error = root
            .release(settled)
            .expect_err("the injected body failure was not returned");
        ensure!(
            format!("{error:#}") == "injected fixture body failure with an attached client",
            "the fixture returned another error than its body's: {error:#}"
        );
        ensure!(!path.exists(), "the failed body's owner was not reaped");
        Ok(())
    }

    #[tokio::test]
    async fn a_body_that_reaped_its_owner_is_settled_without_polling_it_again() -> Result<()> {
        super::super::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline =
            FixtureDeadline::start(super::super::fixture_deadline(1, 0), "served owner fixture");
        let (root, project, options) = fixture()?;
        let settled = deadline
            .serve(
                async |served| {
                    let _gate = crate::spawn_gate::spawning().await;
                    let owner = ServiceOwner::open(options.clone(), &project).await?;
                    served.serve(owner)?;
                    served
                        .retire(&options, None, REAP, "served fixture owner did not reap")
                        .await?;
                    ensure!(
                        served.reap(REAP, "no owner").await.is_err(),
                        "a reaped owner could be awaited again"
                    );
                    Err::<(), _>(anyhow!(
                        "injected fixture body failure after its owner was reaped"
                    ))
                },
                async |served| {
                    ensure!(served.task.is_none(), "the reaped owner is still held");
                    served
                        .retire(&options, None, REAP, "served fixture owner did not reap")
                        .await
                },
            )
            .await;
        let error = root
            .release(settled)
            .expect_err("the injected body failure was not returned");
        ensure!(
            format!("{error:#}") == "injected fixture body failure after its owner was reaped",
            "the fixture returned another error than its body's: {error:#}"
        );
        Ok(())
    }

    /// The fixture deadline elapses while the body holds a client of a live
    /// served owner. The deadline's own error is returned, and the owner is
    /// retired before the root is released: the guard removes the root. The
    /// deadline is expired by the body's signal, once the owner is served
    /// and the client attached, so the test does not depend on timing.
    #[tokio::test]
    async fn an_elapsed_deadline_reports_its_own_error_and_retires_the_live_owner() -> Result<()> {
        super::super::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let backstop = FixtureDeadline::start(
            super::super::fixture_deadline(1, 0),
            "expiring served fixture's backstop",
        );
        let (root, project, options) = fixture()?;
        let path = root.path().to_path_buf();
        let (expire, expired) = tokio::sync::oneshot::channel::<()>();
        let outcome = backstop
            .run(async {
                Ok(serve_until(
                    async {
                        let _ = expired.await;
                    },
                    || anyhow!("expiring served fixture exceeded its deadline"),
                    async |served| {
                        let _gate = crate::spawn_gate::spawning().await;
                        let owner = ServiceOwner::open(options.clone(), &project).await?;
                        served.serve(owner)?;
                        let _client = crate::MemoryStore::open_managed_observed(
                            options.clone(),
                            project.clone(),
                            std::env::current_exe()?,
                        )
                        .1
                        .await?;
                        let _ = expire.send(());
                        std::future::pending::<Result<()>>().await
                    },
                    async |served| {
                        served
                            .retire(&options, None, REAP, "served fixture owner did not reap")
                            .await
                    },
                )
                .await)
            })
            .await?;
        let error = root
            .release(outcome)
            .expect_err("the fixture body never finishes");
        ensure!(
            format!("{error:#}") == "expiring served fixture exceeded its deadline",
            "the fixture returned another error than its deadline's: {error:#}"
        );
        ensure!(
            !path.exists(),
            "the live owner was not retired after the deadline elapsed"
        );
        Ok(())
    }

    /// When the root cannot be released after an elapsed deadline, the
    /// guard's verdict is attached to the deadline's error and the root is
    /// kept; nothing panics. An unrecorded store stands for an owner the
    /// teardown could not retire, without starting an engine.
    #[tokio::test]
    async fn an_elapsed_deadline_attaches_the_guard_verdict_instead_of_panicking() -> Result<()> {
        let root = super::super::tempdir()?;
        let kept = root.path().to_path_buf();
        let (expire, expired) = tokio::sync::oneshot::channel::<()>();
        let outcome = serve_until(
            async {
                let _ = expired.await;
            },
            || anyhow!("verdict fixture exceeded its deadline"),
            async |_served| {
                unrecorded_store(&kept)?;
                let _ = expire.send(());
                std::future::pending::<Result<()>>().await
            },
            async |served: &mut ServedOwner| {
                ensure!(served.task.is_none(), "this fixture serves no owner");
                Ok(())
            },
        )
        .await;
        let error = root
            .release(outcome)
            .expect_err("the fixture body never finishes");
        ensure!(
            error.root_cause().to_string() == "verdict fixture exceeded its deadline"
                && format!("{error:#}").contains("has no quiescence record"),
            "the deadline error did not carry the guard's verdict: {error:#}"
        );
        ensure!(kept.exists(), "an unexplained root must be kept");
        std::fs::remove_dir_all(kept.parent().context("kept root parent")?)?;
        Ok(())
    }
}
