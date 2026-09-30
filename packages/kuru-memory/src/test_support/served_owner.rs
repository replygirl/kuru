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

use crate::{
    OpenOptions,
    service::{ServiceOwner, acquire_maintenance_permit},
};
use anyhow::{Context, Error, Result, anyhow, ensure};
use std::{future::Future, path::Path, time::Duration};
use tokio::{sync::RwLockReadGuard, task::JoinHandle, time::Instant};

/// The owner task a fixture is serving, or nothing once that task has ended.
///
/// A finished task is never polled again: [`ServedOwner::reap`] empties the
/// slot as soon as the task has ended, whatever the owner returned.
pub(crate) struct ServedOwner {
    task: Option<JoinHandle<Result<()>>>,
}

impl ServedOwner {
    /// Serve `owner` on a new task: the fixture's first owner, or a successor
    /// after the previous owner has been reaped.
    pub(crate) fn serve(&mut self, owner: ServiceOwner) -> Result<()> {
        ensure!(
            self.task.is_none(),
            "a fixture owner is still being served; reap it before its successor"
        );
        // Never reached: a fixture owner outlives its last client until the
        // fixture retires it (WP2 seam; WP4 adds `serve_with`).
        self.task = Some(tokio::spawn(
            owner.serve_with(crate::service::ServeKnobs::never_reached()),
        ));
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

    /// Retire the idle owner with a fixture's success-tail calls: acquire the
    /// maintenance permit (within `permit_within` when the fixture bounds it),
    /// await the owner's reap within `reap_within`, then release the permit.
    /// With no owner being served there is nothing to retire.
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
