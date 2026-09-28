//! In-process service owners that a fixture serves beneath its guarded root.
//!
//! A fixture that runs [`ServiceOwner::serve`] on a task must retire that
//! owner before its root drops, or the root's guard ([`super::TempDir`])
//! fails the test. When the retirement ran only at the end of the block that
//! also owned the root, an early `?`, `bail!` or `ensure!` detached the
//! served task and dropped the root under a live engine. The guard then
//! correctly failed the test, but its panic replaced the fixture's own error.
//!
//! So such a fixture keeps its root, its options and its [`ServedOwner`]
//! outside the body being judged. It captures the body's `Result` while the
//! root lives and passes that result to [`settle`], with the success tail's
//! own retirement as the teardown. [`settle`] writes a body error to the
//! test's captured output before the teardown starts, then runs the teardown
//! on every path, with the same calls and bounds as the success path. If the
//! teardown cannot retire the owner (a client task aborted mid-request can
//! legitimately keep the owner busy), the guard still fails the test when the
//! root drops, and its panic replaces the returned error. The printed body
//! error survives in the captured output.

use crate::{
    OpenOptions,
    service::{ServiceOwner, acquire_maintenance_permit},
};
use anyhow::{Context, Result, ensure};
use std::{future::Future, time::Duration};
use tokio::task::JoinHandle;

/// The owner task a fixture is serving, or nothing once that task has ended.
///
/// A finished task is never polled again: [`ServedOwner::reap`] empties the
/// slot as soon as the task has ended, whatever the owner returned.
pub(crate) struct ServedOwner {
    task: Option<JoinHandle<Result<()>>>,
}

impl ServedOwner {
    /// Serve `owner` on a new task.
    pub(crate) fn spawn(owner: ServiceOwner) -> Self {
        Self {
            task: Some(tokio::spawn(owner.serve())),
        }
    }

    /// Serve a successor after the previous owner has been reaped.
    pub(crate) fn serve_successor(&mut self, owner: ServiceOwner) -> Result<()> {
        ensure!(
            self.task.is_none(),
            "a fixture owner is still being served; reap it before its successor"
        );
        self.task = Some(tokio::spawn(owner.serve()));
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
    let teardown = teardown.await;
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
    use anyhow::anyhow;
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
        let deadline = super::super::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let (root, project, options) = fixture()?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let mut served = ServedOwner::spawn(owner);
            let body: Result<()> = async {
                let _client = crate::MemoryStore::open_managed_observed(
                    options.clone(),
                    project.clone(),
                    std::env::current_exe()?,
                )
                .1
                .await?;
                Err(anyhow!(
                    "injected fixture body failure with an attached client"
                ))
            }
            .await;
            let settled = settle(
                body,
                served.retire(&options, None, REAP, "served fixture owner did not reap"),
            )
            .await;
            let error = settled.expect_err("the injected body failure was not returned");
            ensure!(
                format!("{error:#}") == "injected fixture body failure with an attached client",
                "the fixture returned another error than its body's: {error:#}"
            );
            ensure!(
                served.task.is_none(),
                "the failed body's owner was not reaped"
            );
            // The guard fails this test if the owner's engine outlived it.
            drop(root);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("served owner fixture exceeded its {deadline:?} deadline"))?
    }

    #[tokio::test]
    async fn a_body_that_reaped_its_owner_is_settled_without_polling_it_again() -> Result<()> {
        super::super::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline = super::super::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let (root, project, options) = fixture()?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let mut served = ServedOwner::spawn(owner);
            let body: Result<()> = async {
                served
                    .retire(&options, None, REAP, "served fixture owner did not reap")
                    .await?;
                Err(anyhow!(
                    "injected fixture body failure after its owner was reaped"
                ))
            }
            .await;
            let settled = settle(
                body,
                served.retire(&options, None, REAP, "served fixture owner did not reap"),
            )
            .await;
            let error = settled.expect_err("the injected body failure was not returned");
            ensure!(
                format!("{error:#}") == "injected fixture body failure after its owner was reaped",
                "the fixture returned another error than its body's: {error:#}"
            );
            ensure!(
                served.reap(REAP, "no owner").await.is_err(),
                "a reaped owner could be awaited again"
            );
            drop(root);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("served owner fixture exceeded its {deadline:?} deadline"))?
    }
}
