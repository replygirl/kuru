use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub struct StateReadCut(Arc<Backend>);

enum Backend {
    Local(store::StateReadCut),
    Remote(RemoteCut),
}

struct RemoteCut {
    view: RemoteView,
    handle: Uuid,
    provenance: store::StateReadProvenance,
    closed: AtomicBool,
}

impl std::fmt::Debug for StateReadCut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateReadCut")
            .field("provenance", self.provenance())
            .finish()
    }
}

impl MemoryStore {
    pub async fn begin_state_read_cut(&self) -> Result<StateReadCut> {
        let backend = match &self.backend {
            super::Backend::Local(store) => Backend::Local(store.begin_state_read_cut().await?),
            super::Backend::Remote(remote) => {
                let candidate = if remote.candidate.is_some() {
                    Some(crate::service::rpc::StateReadCandidate {
                        branch: remote.pinned_view.clone(),
                        revision: self.revision().await?,
                    })
                } else {
                    None
                };
                let view = remote.fork().await?;
                // Every cut owns its connection, including candidate cuts.
                // Candidate selection is an exact checked read-only ref, never
                // the other attachment's mutable candidate capability.
                let mut attachment = view.attachment.lock().await;
                view.session.ensure_open()?;
                let value = view
                    .checked_call(
                        &mut attachment,
                        ServiceCall::BeginStateReadCut { candidate },
                        None,
                    )
                    .await?;
                drop(attachment);
                let ServiceValue::StateReadCutStarted { handle, provenance } = value else {
                    bail!("memory service returned the wrong state-cut response")
                };
                Backend::Remote(RemoteCut {
                    view,
                    handle,
                    provenance,
                    closed: AtomicBool::new(false),
                })
            }
        };
        Ok(StateReadCut(Arc::new(backend)))
    }
}

impl RemoteCut {
    async fn call(&self, call: ServiceCall) -> Result<ServiceValue> {
        ensure!(!self.closed.load(Ordering::Acquire), "state cut is closed");
        self.view.session.ensure_open()?;
        let mut attachment = self.view.attachment.lock().await;
        ensure!(!self.closed.load(Ordering::Acquire), "state cut is closed");
        self.view.checked_call(&mut attachment, call, None).await
    }
}

impl StateReadCut {
    pub fn provenance(&self) -> &store::StateReadProvenance {
        match self.0.as_ref() {
            Backend::Local(cut) => cut.provenance(),
            Backend::Remote(cut) => &cut.provenance,
        }
    }

    pub async fn get_versioned(&self, key: &str) -> Result<Option<crate::VersionedValue>> {
        store::identifier("state key", key, 1024)?;
        match self.0.as_ref() {
            Backend::Local(cut) => cut.get_versioned(key).await,
            Backend::Remote(cut) => match cut
                .call(ServiceCall::StateReadCutGet {
                    handle: cut.handle,
                    key: key.into(),
                })
                .await?
            {
                ServiceValue::VersionedValue(value) => Ok(value),
                _ => bail!("memory service returned the wrong state-cut value response"),
            },
        }
    }

    pub async fn page(
        &self,
        prefix: &str,
        cursor: Option<store::StateReadCursor>,
    ) -> Result<store::StateReadPage> {
        store::identifier("state prefix", prefix, 1024)?;
        match self.0.as_ref() {
            Backend::Local(cut) => cut.page(prefix, cursor).await,
            Backend::Remote(cut) => match cut
                .call(ServiceCall::StateReadCutPage {
                    handle: cut.handle,
                    prefix: prefix.into(),
                    cursor,
                })
                .await?
            {
                ServiceValue::StateReadPage(page) => Ok(page),
                _ => bail!("memory service returned the wrong state-cut page response"),
            },
        }
    }

    pub async fn close(&self) -> Result<()> {
        match self.0.as_ref() {
            Backend::Local(cut) => cut.close().await,
            Backend::Remote(cut) => {
                let mut attachment = cut.view.attachment.lock().await;
                if cut.closed.load(Ordering::Acquire) {
                    return Ok(());
                }
                let result = if attachment.has_complete_exchange() {
                    cut.view
                        .checked_call(
                            &mut attachment,
                            ServiceCall::CloseStateReadCut { handle: cut.handle },
                            None,
                        )
                        .await
                        .and_then(unit)
                } else {
                    Ok(())
                };
                // On a cancelled exchange InFlight closes this dedicated
                // transport. A retry reaches this terminal teardown rather
                // than replaying a handle on a new attachment/generation.
                attachment.close();
                cut.closed.store(true, Ordering::Release);
                result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate as kuru_memory;
    use crate::test_support::{FixtureDeadline, fixture_deadline};
    use serde_json::json;

    fn remote(cut: &StateReadCut) -> Result<&RemoteCut> {
        match cut.0.as_ref() {
            Backend::Remote(remote) => Ok(remote),
            _ => bail!("fixture requires managed cut"),
        }
    }

    async fn cancelled_exchange(cut: &StateReadCut, close: bool) -> Result<()> {
        let pause = Arc::new(service::rpc::ReplyPause::default());
        remote(cut)?
            .view
            .attachment
            .lock()
            .await
            .pause_after_next_send(pause.clone());
        let mut exchange = Box::pin(async {
            if close {
                cut.close().await
            } else {
                cut.page("reports/", None).await.map(|_| ())
            }
        });
        // The actual completed reply proves the owner no longer holds a query;
        // cancellation now tests transport/handle cleanup, not a timer guess.
        tokio::select! {
            result = &mut exchange => bail!("paused cut exchange finished early: {result:?}"),
            () = pause.replied.notified() => {},
        }
        drop(exchange);
        assert!(
            !remote(cut)?
                .view
                .attachment
                .lock()
                .await
                .has_complete_exchange()
        );
        Ok(())
    }

    #[tokio::test]
    async fn state_read_cut_managed_paging_cancellation_close_and_restart_are_owned() -> Result<()>
    {
        kuru_memory::test_support::closing(async {
            crate::test_support::warm_runtime_cache().await?;
            let deadline =
                FixtureDeadline::start(fixture_deadline(2, 0), "managed immutable state cuts");
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let options = crate::test_support::warmed_open_options(
                root.path().join("private"),
                format!("project/{digest}"),
            )
            .await?;
            let outcome = deadline
                .serve(
                    async |served| {
                        let gate = crate::spawn_gate::spawning().await;
                        let owner = service::ServiceOwner::open(options.clone(), &project).await?;
                        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
                        let mut knobs = service::ServeKnobs::never_reached();
                        knobs.observer = Some(events);
                        served.serve_with(owner, knobs)?;
                        let executable = std::env::current_exe()?;
                        let memory = MemoryStore::open_managed_observed(
                            options.clone(),
                            project.clone(),
                            executable.clone(),
                        )
                        .1
                        .await?;
                        let values: Vec<_> = (0..300)
                            .map(|n| (format!("reports/{n:04}"), json!(n)))
                            .collect();
                        memory.put_many(&values).await?;
                        memory.put("membership", &json!("old")).await?;
                        let cut = memory.begin_state_read_cut().await?;
                        let peer = memory.begin_state_read_cut().await?;
                        let first = cut.page("reports/", None).await?;
                        assert_eq!(first.values.len(), 256);
                        memory.put("reports/0299", &json!("new")).await?;
                        memory.put("membership", &json!("new")).await?;
                        let rest = cut.page("reports/", first.next).await?;
                        assert_eq!(rest.values.len(), 44);
                        assert_eq!(rest.values.last().unwrap().1.value, json!(299));
                        assert_eq!(
                            cut.get_versioned("membership").await?.unwrap().value,
                            json!("old")
                        );
                        let super::super::Backend::Remote(main) = &memory.backend else {
                            bail!("managed fixture is local")
                        };
                        let handle = remote(&cut)?.handle;
                        assert!(
                            main.checked_call(
                                &mut *main.attachment.lock().await,
                                ServiceCall::StateReadCutGet {
                                    handle,
                                    key: "membership".into()
                                },
                                None
                            )
                            .await
                            .is_err()
                        );
                        cut.close().await?;
                        assert_eq!(
                            peer.get_versioned("membership").await?.unwrap().value,
                            json!("old")
                        );
                        peer.close().await?;

                        let candidate = memory.begin_candidate("candidate cut cleanup").await?;
                        let view = candidate.view();
                        view.put("reports/0299", &json!("candidate")).await?;
                        let cancelled = view.begin_state_read_cut().await?;
                        let peer = view.begin_state_read_cut().await?;
                        assert_eq!(cancelled.provenance().branch, candidate.branch());
                        while received.try_recv().is_ok() {}
                        cancelled_exchange(&cancelled, false).await?;
                        drop(cancelled);
                        tokio::time::timeout(crate::server::close_budget(), async {
                            loop {
                                if matches!(
                                    received.recv().await,
                                    Some(service::ServeEvent::AttachmentJoined { .. })
                                ) {
                                    break;
                                }
                            }
                        })
                        .await
                        .context("cancelled candidate cut attachment did not join")?;
                        assert_eq!(
                            peer.get_versioned("reports/0299").await?.unwrap().value,
                            json!("candidate")
                        );
                        view.put("candidate-writer-survives", &json!(true)).await?;
                        peer.close().await?;

                        let interrupted = view.begin_state_read_cut().await?;
                        cancelled_exchange(&interrupted, true).await?;
                        assert!(!remote(&interrupted)?.closed.load(Ordering::Acquire));
                        interrupted.close().await?;
                        assert!(remote(&interrupted)?.closed.load(Ordering::Acquire));
                        interrupted.close().await?;
                        view.put("candidate-close-survives", &json!(true)).await?;
                        let captured = view.revision().await?;
                        view.put("candidate-moved", &json!(true)).await?;
                        let dedicated = main.fork().await?;
                        assert!(
                            dedicated
                                .checked_call(
                                    &mut *dedicated.attachment.lock().await,
                                    ServiceCall::BeginStateReadCut {
                                        candidate: Some(service::rpc::StateReadCandidate {
                                            branch: candidate.branch().into(),
                                            revision: captured
                                        })
                                    },
                                    None
                                )
                                .await
                                .is_err()
                        );
                        dedicated.attachment.lock().await.close();
                        candidate.abandon().await?;
                        drop(view);

                        // A row larger than the small fastpath is still valid in the same
                        // real managed CAS and cut operation envelopes.
                        let large = json!("x".repeat(store::MAX_STATE_BATCH_BYTES + 1024));
                        memory
                            .put_many_conditional(
                                &[("large/value".into(), crate::StateExpectation::Absent)],
                                &[("large/value".into(), large.clone())],
                            )
                            .await?;
                        let large_cut = memory.begin_state_read_cut().await?;
                        assert_eq!(
                            large_cut.page("large/", None).await?.values[0].1.value,
                            large
                        );
                        large_cut.close().await?;
                        let stale = memory.begin_state_read_cut().await?;
                        let stale_handle = remote(&stale)?.handle;
                        memory.close().await?;
                        let _gate = served
                            .restart(
                                gate,
                                &options,
                                &project,
                                crate::server::close_budget(),
                                "old state-cut owner did not reap",
                            )
                            .await?;
                        assert!(stale.get_versioned("membership").await.is_err());
                        let fresh = MemoryStore::open_managed_observed(
                            options.clone(),
                            project.clone(),
                            executable,
                        )
                        .1
                        .await?;
                        let super::super::Backend::Remote(new) = &fresh.backend else {
                            bail!("successor is local")
                        };
                        assert!(
                            new.checked_call(
                                &mut *new.attachment.lock().await,
                                ServiceCall::StateReadCutGet {
                                    handle: stale_handle,
                                    key: "membership".into()
                                },
                                None
                            )
                            .await
                            .is_err()
                        );
                        fresh.close().await?;
                        Ok::<(), anyhow::Error>(())
                    },
                    async |served| {
                        served
                            .retire(
                                &options,
                                None,
                                crate::server::close_budget(),
                                "state-cut fixture owner did not reap",
                            )
                            .await
                    },
                )
                .await;
            root.release(outcome)
        })
        .await
    }
}
