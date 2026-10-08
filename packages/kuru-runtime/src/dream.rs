use anyhow::{Context, Result, ensure};
use futures::future::join_all;
use kuru_core::{
    ActorPhase, Config, Mode, Part, ToolSpec, canonical_peer_instruction,
    validate_consolidation_plan,
};
use kuru_memory::{
    Candidate, CandidateConflict, CandidateReconciliationResolution, CandidateReconciliationResult,
    MemoryStore, StateExpectation,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    Event,
    engine::{
        CancellationToken, CandidatePromotionStatus, CandidateResolutionRequired, Harness,
        OfferedTools, PendingCandidateResolution, PendingPublication, PublicationProof,
        PublicationScope, Session, ToolHookAdmission, Topology, checked_state_keys,
        prepared_actor_namespaces, run_post_tool_hooks, spec, turn_was_cancelled, user,
        validate_topology_with_profile,
    },
    topology_state::{self, MembershipRecord},
};

#[cfg(test)]
use crate::engine::read_topology;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum DreamProposal {
    Add {
        name: String,
        role: String,
        instruction: String,
    },
    Retire {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DreamReport {
    pub accepted: Vec<DreamProposal>,
    pub rejected: Vec<String>,
    pub summaries: usize,
}

impl Harness {
    pub async fn dream(&mut self) -> Result<DreamReport> {
        let cancellation = CancellationToken::new();
        self.dream_controlled(&cancellation).await
    }

    /// Run dreaming while observing an explicit foreground cancellation signal.
    pub async fn dream_controlled(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<DreamReport> {
        self.dream_controlled_inner(cancellation, true).await
    }

    pub(crate) async fn dream_controlled_during_turn(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<DreamReport> {
        self.dream_controlled_inner(cancellation, false).await
    }

    async fn dream_controlled_inner(
        &mut self,
        cancellation: &CancellationToken,
        reset_context: bool,
    ) -> Result<DreamReport> {
        let _driver_work = self.driver_work(cancellation)?;
        if reset_context {
            self.reset_context_snapshot();
        }
        #[cfg(test)]
        self.step_timings.mark("dream started");
        cancellation.check()?;
        self.reconcile().await?;
        #[cfg(test)]
        self.step_timings.mark("reconcile finished");
        cancellation.check()?;
        let _dream_lease = cancellation.wait(self.memory.acquire_dream_lease()).await?;
        #[cfg(test)]
        self.step_timings.mark("dream lease acquired");
        cancellation.check()?;
        self.operation_id = format!("dream-{}", Uuid::new_v4());
        let hook_host = self.hook_host();
        let hook_budget = hook_host.budget();
        let dream_tools = vec![dream_tool()];
        let candidate = self.memory.begin_candidate("dream").await?;
        #[cfg(test)]
        self.step_timings.mark("dream candidate begun");
        // A cancelled future drops local stack state without running the
        // error path below. Keep the exact ref on Harness before the first
        // candidate write or promotion await.
        self.pending_candidate = Some(candidate.clone());
        let outcome = async {
            cancellation.check()?;
            let memory = candidate.view();
            let snapshot = topology_state::load(&memory, &checked_state_keys(&self.scope, &self.profile)?, &self.profile, None, None).await?;
            validate_topology_with_profile(&snapshot.topology, &self.config, &self.profile)?;
            self.prepare_candidate_actors(&snapshot.topology)?;
            let active = snapshot.topology.parts.iter().filter(|part| part.active).map(|part| part.id.clone()).collect::<Vec<_>>();
            let plan = self.profile.memory.consolidation_plan(&active);
            validate_consolidation_plan(&plan, &active.iter().cloned().collect())?;
            self.emit_event(Event::Dream { actor: "pool".into(), detail: "parts are consolidating their own memories".into() });
            let ids = &plan.participants;
            let replies = join_all(ids.iter().map(|id| self.ask_in_controlled_with_invocation(&memory, &snapshot.topology, id,
                vec![user(&plan.prompt)],
                (&plan.phase, ActorPhase::Dream), dream_tools.clone(), cancellation))).await;
            let mut report = DreamReport::default();
            let mut proposals = vec![];
            for (id, reply) in ids.iter().cloned().zip(replies) {
                match reply {
                    Err(error) if error.is::<crate::actor::MemoryFailure>() || error.is::<crate::actor::AccountingFailure>() => return Err(error),
                    Err(error) if turn_was_cancelled(&error) => return Err(error),
                    Err(error) => report.rejected.push(format!("{id}: {error:#}")),
                    Ok((reply, invocation_id)) => {
                        let summary = reply.text_projection();
                        if !summary.trim().is_empty() {
                            cancellation.check()?;
                            memory
                                .append(
                                    &format!("{}/notes", self.checked_namespace(&id)?),
                                    "dream",
                                    &crate::actor::truncate_text(&summary, 8192),
                                )
                                .await?;
                            cancellation.check()?;
                            report.summaries += 1;
                        }
                        for (index, original) in reply.calls().into_iter().enumerate() {
                            // The authored proposal cap is checked before any hook sees
                            // the call; the shared pre-tool admission then settles a call
                            // outside the offered dream tool before any hook runs.
                            let admission = if index >= plan.max_proposals_per_part {
                                ToolHookAdmission::Dispatch(original)
                            } else {
                                self.run_pre_tool_hooks(
                                    &hook_host,
                                    &hook_budget,
                                    (&id, &invocation_id, None),
                                    OfferedTools::Exact(&dream_tools),
                                    original,
                                    cancellation,
                                ).await?
                            };
                            let (call, pre_settled) = match admission {
                                ToolHookAdmission::Dispatch(call) => (call, None),
                                ToolHookAdmission::Settled(call, result) => (call, Some(result)),
                            };
                            let result = if index >= plan.max_proposals_per_part {
                                Err(anyhow::anyhow!(if plan.max_proposals_per_part == 2 {
                                    "at most two dream proposals are accepted per part".to_string()
                                } else {
                                    format!(
                                        "at most {} dream proposals are accepted per part",
                                        plan.max_proposals_per_part
                                    )
                                }))
                            } else if let Some(result) = pre_settled {
                                result.and_then(|_| Err(anyhow::anyhow!("pre-tool hook did not settle a dream proposal")))
                            } else if call.name != "dream_suggest" {
                                // Unreachable: admission settles unoffered calls above.
                                Err(anyhow::anyhow!("tool is not offered in this phase"))
                            } else {
                                serde_json::from_value::<DreamProposal>(call.arguments.clone())
                                    .map_err(Into::into)
                            };
                            let (outcome, succeeded) = match result {
                                Ok(proposal) => {
                                    if matches!(&proposal, DreamProposal::Retire { id: target } if target != &id)
                                    {
                                        let error =
                                            format!("{id}: parts may only retire themselves");
                                        report.rejected.push(error.clone());
                                        (error, false)
                                    } else {
                                        proposals.push(proposal);
                                        ("proposal submitted for validation".to_string(), true)
                                    }
                                }
                                Err(error) => {
                                    let error = format!("{id}: {error}");
                                    report.rejected.push(error.clone());
                                    (error, false)
                                }
                            };
                            cancellation.check()?;
                            memory
                                .append(
                                    &self.checked_namespace(&id)?,
                                    "tool",
                                    &json!({"call_id":call.id,"output":outcome}).to_string(),
                                )
                                .await?;
                            cancellation.check()?;
                            let settled: Result<String> = if succeeded {
                                Ok(outcome)
                            } else {
                                Err(anyhow::anyhow!(outcome))
                            };
                            let post = cancellation.wait(async {
                                Ok(run_post_tool_hooks(
                                    hook_host.clone(),
                                    hook_budget.clone(),
                                    id.clone(),
                                    invocation_id.clone(),
                                    None,
                                    call.clone(),
                                    &settled,
                                ).await)
                            }).await?;
                            self.finish_post_tool_hooks(
                                &memory,
                                &id,
                                &invocation_id,
                                None,
                                &call,
                                post,
                            ).await?;
                        }
                    }
                }
            }
            let (topology, changes) = self.plan_dream(&snapshot.topology, proposals)?;
            report.accepted = changes.accepted;
            report.rejected.extend(changes.rejected);
            self.finish_dream(&candidate, &snapshot.topology, snapshot.expectation, topology, &report, cancellation)
                .await?;
            self.emit_event(Event::Dream {
                actor: "pool".into(),
                detail: format!(
                    "{} summaries, {} changes, {} rejected proposals",
                    report.summaries,
                    report.accepted.len(),
                    report.rejected.len()
                ),
            });
            Ok(report)
        }
        .await;
        // Hook trees whose callers were cancelled finish cleanup first.
        self.await_hook_cleanup().await;
        self.resolve_candidate_outcome(candidate, outcome).await
    }

    pub async fn apply_dream(&mut self, proposals: Vec<DreamProposal>) -> Result<DreamReport> {
        self.reconcile().await?;
        let _dream_lease = self.memory.acquire_dream_lease().await?;
        let candidate = self.memory.begin_candidate("dream").await?;
        self.pending_candidate = Some(candidate.clone());
        let cancellation = CancellationToken::new();
        let outcome = async {
            let snapshot = topology_state::load(
                &candidate.view(),
                &checked_state_keys(&self.scope, &self.profile)?,
                &self.profile,
                None,
                None,
            )
            .await?;
            validate_topology_with_profile(&snapshot.topology, &self.config, &self.profile)?;
            let (topology, report) = self.plan_dream(&snapshot.topology, proposals)?;
            self.finish_dream(
                &candidate,
                &snapshot.topology,
                snapshot.expectation,
                topology,
                &report,
                &cancellation,
            )
            .await?;
            Ok(report)
        }
        .await;
        self.resolve_candidate_outcome(candidate, outcome).await
    }

    async fn finish_dream(
        &mut self,
        candidate: &Candidate,
        original: &Topology,
        expectation: StateExpectation,
        topology: Topology,
        report: &DreamReport,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        cancellation.check()?;
        let memory = candidate.view();
        let keys = checked_state_keys(&self.scope, &self.profile)?;
        let actor_namespaces = prepared_actor_namespaces(&self.scope, &self.profile, &topology)?;
        let mut extra = vec![(
            format!("{}/{}/last-dream", self.scope, self.config.mode),
            json!({"id":Uuid::new_v4(), "base":candidate.base()}),
        )];
        if !report.accepted.is_empty() {
            extra.push((
                keys.dream_undo.clone(),
                serde_json::to_value(MembershipRecord::from_topology(original))?,
            ));
            extra.push((
                keys.membership.clone(),
                serde_json::to_value(MembershipRecord::from_topology(&topology))?,
            ));
        }
        let updates = extra;
        cancellation.check()?;
        if report.accepted.is_empty() {
            memory.put_many(&updates).await?;
        } else {
            memory
                .put_many_conditional(&[(keys.membership, expectation)], &updates)
                .await?;
        }
        cancellation.check()?;
        memory
            .append(
                &format!("{}/{}/dream-log", self.scope, self.config.mode),
                "dream",
                &serde_json::to_string(report)?,
            )
            .await?;
        cancellation.check()?;
        let target = memory.revision().await?;
        self.pending_publication = Some(PendingPublication {
            config: self.config.clone(),
            profile: self.profile.clone(),
            actor_namespaces,
            topology,
            session: self.session.clone(),
            updates,
            proof: PublicationProof::CandidatePromotion {
                base: candidate.base().to_owned(),
                target: target.clone(),
                report: report.clone(),
                status: CandidatePromotionStatus::Pending,
                reconciliation_attempts: 0,
            },
            scope: PublicationScope::Membership { session: false },
        });
        #[cfg(test)]
        self.pause_before_dream_promotion().await?;
        self.drive_dream_promotion(true, cancellation).await?;
        #[cfg(test)]
        self.pause_after_memory_write().await?;
        self.publish_pending_dream().await?;
        Ok(())
    }

    /// Continue only the already staged dream. The retained counter bounds live
    /// movement retries across cancellation and recovery as well as one call.
    pub(crate) async fn drive_dream_promotion(
        &mut self,
        mut reconcile_first: bool,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        loop {
            cancellation.check()?;
            let candidate = self
                .pending_candidate
                .clone()
                .context("staged dream lost its checked candidate")?;
            let Some(PendingPublication {
                proof:
                    PublicationProof::CandidatePromotion {
                        base,
                        target,
                        reconciliation_attempts,
                        ..
                    },
                ..
            }) = self.pending_publication.as_ref()
            else {
                anyhow::bail!("staged dream lost its publication proof");
            };
            ensure!(candidate.base() == base, "dream candidate base changed");
            let target = target.clone();
            if reconcile_first {
                if *reconciliation_attempts >= 3 {
                    self.mark_dream_open(true)?;
                    return Err(CandidateResolutionRequired(
                        "live memory kept moving; dream candidate remains open for explicit resolution",
                    ).into());
                }
                let live = self.memory.revision().await?;
                if let Some(PendingPublication {
                    proof:
                        PublicationProof::CandidatePromotion {
                            status,
                            reconciliation_attempts,
                            ..
                        },
                    ..
                }) = &mut self.pending_publication
                {
                    *reconciliation_attempts += 1;
                    *status = CandidatePromotionStatus::Reconciling {
                        from: target.clone(),
                        live: live.clone(),
                    };
                }
                let outcome = candidate.reconcile_with_live(&target, &live).await?;
                match outcome.result {
                    CandidateReconciliationResult::Reconciled { head, base }
                    | CandidateReconciliationResult::Unchanged { head, base } => {
                        ensure!(
                            base == live,
                            "dream reconciliation changed its selected live base"
                        );
                        self.adopt_reconciled_dream(
                            outcome
                                .candidate
                                .context("reconciled dream lost its fresh handle")?,
                            head,
                        )?;
                    }
                    CandidateReconciliationResult::LiveMoved { .. } => {
                        // A terminal no-effect reply is not an in-flight merge.
                        // Keep its consumed attempt, but discard that pending tuple.
                        if let Some(PendingPublication {
                            proof: PublicationProof::CandidatePromotion { status, .. },
                            ..
                        }) = &mut self.pending_publication
                        {
                            *status = CandidatePromotionStatus::Pending;
                        }
                        continue;
                    }
                    CandidateReconciliationResult::Conflict { .. } => {
                        self.mark_dream_open(true)?;
                        return Err(CandidateConflict.into());
                    }
                }
            }
            cancellation.check()?;
            let candidate = self
                .pending_candidate
                .clone()
                .context("reconciled dream lost its checked candidate")?;
            let Some(PendingPublication {
                proof: PublicationProof::CandidatePromotion { target, .. },
                ..
            }) = self.pending_publication.as_ref()
            else {
                anyhow::bail!("reconciled dream lost its publication proof");
            };
            let target = target.clone();
            // Adoption above replaces handle, effective base and target before
            // the facade captures the next exact promotion tuple.
            #[cfg(test)]
            if let Some(barrier) = self.dream_transition_reply_pause.take() {
                candidate
                    .view()
                    .fixture_pause_next_service_reply(&barrier)
                    .await?;
            }
            if let Some(PendingPublication {
                proof: PublicationProof::CandidatePromotion { status, .. },
                ..
            }) = &mut self.pending_publication
            {
                // From this exact send boundary a lost reply must use typed
                // transition recovery, never replay the checked promotion.
                *status = CandidatePromotionStatus::Pending;
            }
            match candidate.promote_exact(&target).await {
                Ok(promoted) => {
                    ensure!(
                        promoted == target,
                        "dream promoted a different candidate revision"
                    );
                    if let Some(PendingPublication {
                        proof: PublicationProof::CandidatePromotion { status, .. },
                        ..
                    }) = &mut self.pending_publication
                    {
                        *status = CandidatePromotionStatus::Confirmed(promoted);
                    }
                    return Ok(());
                }
                Err(error) if error.is::<CandidateConflict>() => reconcile_first = true,
                Err(error) => return Err(error),
            }
        }
    }

    fn adopt_reconciled_dream(&mut self, candidate: Candidate, head: String) -> Result<()> {
        let previous = self
            .pending_candidate
            .as_ref()
            .context("reconciled dream lost its original branch")?;
        ensure!(
            previous.branch() == candidate.branch(),
            "dream reconciliation changed its branch"
        );
        let Some(PendingPublication {
            proof:
                PublicationProof::CandidatePromotion {
                    base,
                    target,
                    status,
                    ..
                },
            ..
        }) = &mut self.pending_publication
        else {
            anyhow::bail!("reconciled dream lost its publication proof");
        };
        *base = candidate.base().to_owned();
        *target = head;
        *status = CandidatePromotionStatus::Reconciled;
        self.pending_candidate = Some(candidate);
        Ok(())
    }

    fn mark_dream_open(&mut self, conflict: bool) -> Result<()> {
        let Some(PendingPublication {
            proof: PublicationProof::CandidatePromotion { status, .. },
            ..
        }) = &mut self.pending_publication
        else {
            anyhow::bail!("open dream lost its publication proof");
        };
        *status = if conflict {
            CandidatePromotionStatus::OpenConflict
        } else {
            CandidatePromotionStatus::OpenUnchanged
        };
        self.pending_candidate_resolution = PendingCandidateResolution::Explicit;
        Ok(())
    }

    pub(crate) async fn recover_dream_reconciliation(
        &mut self,
        candidate: &Candidate,
        from: &str,
        live: &str,
    ) -> Result<()> {
        let Some(recovery) = candidate.recover_reconciliation(from, live).await? else {
            // A drop before the facade installed a request cannot prove a merge.
            let inspected = self.memory.candidate_ref_status(candidate.branch()).await?;
            ensure!(
                inspected.head.as_deref() == Some(from)
                    && inspected.base.as_deref() == Some(candidate.base()),
                "unsent dream reconciliation changed its exact candidate"
            );
            match inspected.state {
                kuru_memory::CandidateRefState::OpenUnchanged => self.mark_dream_open(false)?,
                kuru_memory::CandidateRefState::OpenConflict => self.mark_dream_open(true)?,
                _ => anyhow::bail!("dream reconciliation remains unproved"),
            }
            return Err(CandidateResolutionRequired(
                "dream reconciliation was not committed; explicit resolution is required",
            )
            .into());
        };
        match recovery.resolution {
            CandidateReconciliationResolution::Committed(head) => {
                ensure!(
                    recovery.candidate.base() == live,
                    "recovered dream changed its selected live base"
                );
                self.adopt_reconciled_dream(recovery.candidate, head)
            }
            CandidateReconciliationResolution::NotCommitted(status) => {
                ensure!(
                    status.head.as_deref() == Some(from),
                    "uncommitted dream head changed"
                );
                self.adopt_reconciled_dream(recovery.candidate, from.to_owned())?;
                match status.state {
                    kuru_memory::CandidateRefState::OpenUnchanged => self.mark_dream_open(false)?,
                    kuru_memory::CandidateRefState::OpenConflict => self.mark_dream_open(true)?,
                    _ => anyhow::bail!("uncommitted dream is not an exact open candidate"),
                }
                Err(CandidateResolutionRequired(
                    "dream reconciliation was not committed; explicit resolution is required",
                )
                .into())
            }
        }
    }

    async fn resolve_candidate_outcome(
        &mut self,
        candidate: Candidate,
        outcome: Result<DreamReport>,
    ) -> Result<DreamReport> {
        match outcome {
            Ok(report) => Ok(report),
            Err(error) => {
                // A lost candidate write or transition reply cannot authorize
                // abandonment. Keep the exact ref and staged publication for
                // typed reconciliation on the next controlled operation.
                ensure!(
                    self.pending_candidate
                        .as_ref()
                        .is_some_and(|retained| { retained.branch() == candidate.branch() }),
                    "dream lost its exact pending candidate"
                );
                // Staged reconciliation/promotion must retain its exact proof.
                // A definite conflict is already marked Explicit by the driver.
                if self.pending_publication.is_none() {
                    self.pending_candidate_resolution =
                        PendingCandidateResolution::AutomaticCleanup;
                }
                Err(error)
            }
        }
    }

    /// Explicitly resolve a proven open dream ref without replaying a lost
    /// write or promotion. A pending promotion must first receive its exact
    /// typed outcome; an incomplete abandonment remains fenced for lookup.
    pub async fn abandon_pending_dream(&mut self) -> Result<()> {
        self.abandon_pending_dream_checked(None).await
    }

    /// Resolve the runtime's retained candidate only when it is the exact ref
    /// and head selected by the user. This is the attached-handle route for a
    /// pending dream; unowned historical refs use selected owner resolution.
    pub async fn abandon_pending_dream_exact(
        &mut self,
        branch: &str,
        base: &str,
        head: &str,
    ) -> Result<()> {
        let candidate = self
            .pending_candidate
            .as_ref()
            .context("no retained dream candidate matches the selected ref")?;
        ensure!(
            candidate.branch() == branch && candidate.base() == base,
            "selected dream candidate identity changed"
        );
        let inspected = self.memory.candidate_ref_status(branch).await?;
        ensure!(
            matches!(
                inspected.state,
                kuru_memory::CandidateRefState::OpenUnchanged
                    | kuru_memory::CandidateRefState::OpenConflict
            ) && inspected.base.as_deref() == Some(base)
                && inspected.head.as_deref() == Some(head),
            "selected dream candidate changed since inspection"
        );
        self.abandon_pending_dream_checked(Some(head)).await
    }

    async fn abandon_pending_dream_checked(&mut self, expected_head: Option<&str>) -> Result<()> {
        let open_promotion = matches!(
            self.pending_publication
                .as_ref()
                .map(|pending| &pending.proof),
            Some(PublicationProof::CandidatePromotion {
                status: CandidatePromotionStatus::OpenUnchanged
                    | CandidatePromotionStatus::OpenConflict
                    | CandidatePromotionStatus::Reconciled,
                ..
            })
        );
        ensure!(
            self.pending_candidate_resolution != PendingCandidateResolution::AbandonSent,
            "previous dream abandonment must resolve before another request"
        );
        ensure!(
            open_promotion
                || (self.pending_publication.is_none()
                    && self.pending_candidate_resolution == PendingCandidateResolution::Explicit),
            "dream candidate must have a proven open outcome before explicit abandonment"
        );
        let candidate = self
            .pending_candidate
            .as_ref()
            .context("proven open dream candidate lost its checked handle")?
            .clone();
        self.pending_candidate_resolution = PendingCandidateResolution::AbandonSent;
        #[cfg(test)]
        self.pause_before_dream_abandon().await?;
        let result = if let Some(head) = expected_head {
            candidate.abandon_exact(head).await
        } else {
            candidate.abandon().await
        };
        if let Err(error) = result {
            if error.is::<CandidateConflict>() {
                // Exact-head mismatch or a typed owner refusal happened before
                // any transition. No outcome query is needed for this attempt.
                self.pending_candidate_resolution = PendingCandidateResolution::Explicit;
            }
            return Err(error);
        }
        self.pending_candidate = None;
        self.pending_publication = None;
        self.pending_candidate_resolution = PendingCandidateResolution::AutomaticCleanup;
        self.restore_live_actors()?;
        Ok(())
    }

    fn plan_dream(
        &self,
        original: &Topology,
        proposals: Vec<DreamProposal>,
    ) -> Result<(Topology, DreamReport)> {
        ensure!(
            proposals.len() <= self.config.max_parts * 2,
            "too many dream proposals"
        );
        let mut topology = original.clone();
        let mut report = DreamReport::default();
        for proposal in proposals {
            let mut candidate = topology.clone();
            let validation = match &proposal {
                DreamProposal::Add {
                    name,
                    role,
                    instruction,
                } => {
                    if name.trim().is_empty() || name.len() > 128 {
                        Err(anyhow::anyhow!(
                            "new part requires a short name and 1–8192 byte instruction"
                        ))
                    } else if !candidate.parts.iter().any(|p| &p.role == role) {
                        Err(anyhow::anyhow!(
                            "new parts must use an existing framework role"
                        ))
                    } else if candidate
                        .parts
                        .iter()
                        .any(|p| p.active && p.name.eq_ignore_ascii_case(name))
                    {
                        Err(anyhow::anyhow!("active part name already exists"))
                    } else {
                        canonical_peer_instruction(instruction).map(|instruction| {
                            candidate.parts.push(Part {
                                id: Uuid::new_v4().to_string(),
                                name: name.clone(),
                                role: role.clone(),
                                instruction,
                                active: true,
                            });
                        })
                    }
                }
                DreamProposal::Retire { id } => {
                    if let Some(part) = candidate.parts.iter_mut().find(|p| &p.id == id && p.active)
                    {
                        part.active = false;
                        candidate.focus = None;
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("retirement target is not active"))
                    }
                }
            }
            .and_then(|()| validate_topology_with_profile(&candidate, &self.config, &self.profile));
            match validation {
                Ok(()) => {
                    topology = candidate;
                    report.accepted.push(proposal);
                }
                Err(error) => report.rejected.push(format!("{proposal:?}: {error}")),
            }
        }
        Ok((topology, report))
    }

    pub async fn undo_dream(&mut self) -> Result<()> {
        self.reconcile().await?;
        let _dream_lease = self.memory.acquire_dream_lease().await?;
        let undo = prepare_undo_dream(
            &self.config,
            &self.scope,
            &self.memory,
            Some(&self.session.id),
            Some(&self.profile),
        )
        .await?;
        self.persist_membership(
            undo.topology,
            undo.expectation,
            vec![(undo.key, json!(null))],
            false,
            None,
        )
        .await
    }
}

/// Undo the latest dream membership change without starting a provider, tool
/// host, actor, or harness. A resumed session selects its saved mode; otherwise
/// the finalized configuration selects the mode.
pub async fn undo_dream(
    config: &Config,
    scope: &str,
    memory: &MemoryStore,
    resume: Option<&str>,
) -> Result<()> {
    memory.ensure_project_scope(scope)?;
    let scope = memory.history_scope(scope)?;
    let _dream_lease = memory.acquire_dream_lease().await?;
    let undo = prepare_undo_dream(config, scope, memory, resume, None).await?;
    memory
        .put_many_conditional(
            &[(undo.membership_key.clone(), undo.expectation)],
            &[
                (
                    undo.membership_key,
                    serde_json::to_value(MembershipRecord::from_topology(&undo.topology))?,
                ),
                (undo.key, json!(null)),
            ],
        )
        .await?;
    memory.reconcile().await?;
    Ok(())
}

struct UndoDream {
    membership_key: String,
    expectation: StateExpectation,
    topology: Topology,
    key: String,
}

async fn prepare_undo_dream(
    config: &Config,
    scope: &str,
    memory: &MemoryStore,
    resume: Option<&str>,
    profile: Option<&kuru_core::ModeProfile>,
) -> Result<UndoDream> {
    config.validate()?;
    memory.reconcile().await?;
    let mode = resume_mode(config, scope, memory, resume).await?;
    let mut config = config.clone();
    config.mode = mode;
    config.validate()?;
    let builtin;
    let profile = if let Some(profile) = profile {
        ensure!(
            profile.mode == mode,
            "mode profile does not match dream undo mode"
        );
        profile
    } else {
        builtin = kuru_core::ModeProfile::builtin(mode);
        &builtin
    };
    let keys = checked_state_keys(scope, profile)?;
    let snapshot = topology_state::load(memory, &keys, profile, None, None).await?;
    let current = snapshot.topology;
    let key = keys.dream_undo;
    let value = memory
        .get(&key)
        .await?
        .filter(|value| !value.is_null())
        .context("no dreaming change to undo")?;
    let previous = MembershipRecord::decode(value, true)?;
    let previous = Topology {
        parts: previous.parts,
        relationships: previous.relationships,
        states: current.states.clone(),
        focus: None,
    };
    validate_topology_with_profile(&previous, &config, profile)?;
    let restored = restore_topology(previous, &current);
    validate_topology_with_profile(&restored, &config, profile)?;
    Ok(UndoDream {
        membership_key: keys.membership,
        expectation: snapshot.expectation,
        topology: restored,
        key,
    })
}

async fn resume_mode(
    config: &Config,
    scope: &str,
    memory: &MemoryStore,
    resume: Option<&str>,
) -> Result<Mode> {
    let Some(resume) = resume else {
        return Ok(config.mode);
    };
    let session: Session = serde_json::from_value(
        memory
            .get(&format!("{scope}/session/{resume}"))
            .await?
            .context("session not found in this project")?,
    )?;
    Ok(session.mode)
}

fn restore_topology(mut previous: Topology, current: &Topology) -> Topology {
    // Keep newly created identities archived so their memories remain inspectable.
    for part in &current.parts {
        if !previous
            .parts
            .iter()
            .any(|previous_part| previous_part.id == part.id)
        {
            let mut archived = part.clone();
            archived.active = false;
            previous.parts.push(archived);
        }
    }
    previous
}

fn dream_tool() -> ToolSpec {
    // oneOf allows providers to preserve validation rather than using prose-only JSON.
    let mut tool = spec(
        "dream_suggest",
        "Suggest adding a complementary peer or retiring yourself; histories are retained.",
        json!({}),
        &[],
    );
    tool.parameters = json!({"type":"object","oneOf":[
        {"type":"object","properties":{"action":{"const":"add","type":"string"},"name":{"type":"string"},"role":{"type":"string"},"instruction":{"type":"string"}},"required":["action","name","role","instruction"],"additionalProperties":false},
        {"type":"object","properties":{"action":{"const":"retire","type":"string"},"id":{"type":"string"}},"required":["action","id"],"additionalProperties":false}
    ]});
    tool
}

#[cfg(test)]
mod cancellation_tests {
    use std::sync::Arc;

    use kuru_connectors::DemoProvider;
    use kuru_core::{Config, Mode};
    use kuru_memory::MemoryStore;

    use super::*;

    fn config() -> Config {
        Config {
            mode: Mode::Freudian,
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        }
    }

    struct SerialDreamProvider {
        entered: tokio::sync::mpsc::UnboundedSender<()>,
        release: tokio::sync::Semaphore,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl kuru_connectors::Provider for SerialDreamProvider {
        async fn models(&self) -> Result<Vec<kuru_core::ModelInfo>> {
            Ok(vec![])
        }

        async fn complete(
            &self,
            request: kuru_core::CompletionRequest,
        ) -> Result<kuru_core::Completion> {
            ensure!(request.instructions.contains("Phase: dream"));
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.entered
                .send(())
                .map_err(|_| anyhow::anyhow!("serial dream observer disappeared"))?;
            self.release.acquire().await?.forget();
            Ok(kuru_core::Completion::from_legacy(
                "owned serialized summary",
                vec![],
                0,
                0,
            ))
        }

        async fn stream(
            &self,
            request: kuru_core::CompletionRequest,
            sink: &mut dyn kuru_connectors::ProviderSink,
        ) -> Result<()> {
            sink.emit(kuru_connectors::ProviderEvent::Completed(
                self.complete(request).await?,
            ))
            .await
        }
    }

    #[tokio::test]
    async fn controlled_dreams_serialize_inference_while_ordinary_writes_progress() -> Result<()> {
        kuru_memory::test_support::closing(async {
            use futures::FutureExt as _;
            let project_dir = tempfile::tempdir()?;
            let project = project_dir.path().canonicalize()?;
            let data = kuru_memory::test_support::tempdir()?;
            let options = kuru_memory::test_support::warmed_open_options(data.path().into(), crate::project_scope(&project)?).await?;
            let executable = options.supervisor.clone().context("serial dream supervisor absent")?;
            let mut opened = Vec::new();
            let mut harnesses = Vec::new();
            let outcome = std::panic::AssertUnwindSafe(async {
                let first_memory = MemoryStore::open_managed_observed(options.clone(), project.clone(), executable.clone()).1.await?;
                opened.push(first_memory.clone());
                let second_memory = MemoryStore::open_managed_observed(options.clone(), project.clone(), executable).1.await?;
                opened.push(second_memory.clone());
                let (first_entered, mut first_events) = tokio::sync::mpsc::unbounded_channel();
                let (second_entered, mut second_events) = tokio::sync::mpsc::unbounded_channel();
                let first_provider = Arc::new(SerialDreamProvider { entered: first_entered, release: tokio::sync::Semaphore::new(0), calls: std::sync::atomic::AtomicUsize::new(0) });
                let second_provider = Arc::new(SerialDreamProvider { entered: second_entered, release: tokio::sync::Semaphore::new(0), calls: std::sync::atomic::AtomicUsize::new(0) });
                harnesses.push(Harness::new(config(), &project, first_memory.clone(), first_provider.clone(), None).await?);
                harnesses.push(Harness::new(config(), &project, second_memory.clone(), second_provider.clone(), None).await?);
                let (left, right) = harnesses.split_at_mut(1);
                let first = &mut left[0];
                let second = &mut right[0];
                let second_bound = second.memory.clone();
                let participants = first.topology.parts.len();
                let mut first_work = Box::pin(first.dream());
                for _ in 0..participants {
                    tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                        tokio::select! {
                            event = first_events.recv() => event.context("first provider events closed"),
                            result = &mut first_work => anyhow::bail!("first dream returned before held inference: {result:?}"),
                        }
                    }).await.context("first provider did not enter held inference")??;
                }
                let mut second_work = Box::pin(second.dream());
                tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                    tokio::select! {
                        result = second_bound.fixture_wait_for_dream_lease_refusal() => result,
                        result = &mut second_work => anyhow::bail!("second dream returned before owned lease refusal: {result:?}"),
                        result = &mut first_work => anyhow::bail!("first held dream unexpectedly returned: {result:?}"),
                    }
                }).await.context("second dream did not reach its owned lease wait")??;
                ensure!(second_provider.calls.load(std::sync::atomic::Ordering::SeqCst) == 0);
                second_memory.append("ordinary-live", "user", "while first inference held").await?;
                ensure!(second_provider.calls.load(std::sync::atomic::Ordering::SeqCst) == 0);
                first_provider.release.add_permits(participants);
                ensure!(first_work.await?.summaries == participants);
                for _ in 0..participants {
                    tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                        tokio::select! {
                            event = second_events.recv() => event.context("second provider events closed"),
                            result = &mut second_work => anyhow::bail!("second dream returned before held inference: {result:?}"),
                        }
                    }).await.context("released lease did not admit second inference")??;
                }
                second_provider.release.add_permits(participants);
                ensure!(second_work.await?.summaries == participants);
                ensure!(first_provider.calls.load(std::sync::atomic::Ordering::SeqCst) == participants);
                ensure!(second_provider.calls.load(std::sync::atomic::Ordering::SeqCst) == participants);
                ensure!(first_memory.history("ordinary-live", 10).await?[0].plain_text() == Some("while first inference held"));
                Ok::<(), anyhow::Error>(())
            }).catch_unwind().await;
            let mut cleanup_errors = Vec::new();
            for harness in &mut harnesses {
                if let Err(error) = harness.shutdown(false).await { cleanup_errors.push(format!("shutdown: {error:#}")); }
                opened.push(harness.memory.clone());
                if let Some(candidate) = &harness.pending_candidate { opened.push(candidate.view()); }
            }
            for memory in opened { if let Err(error) = memory.close().await { cleanup_errors.push(format!("close: {error:#}")); } }
            if let Err(error) = kuru_memory::test_support::await_managed_quiescence(&options).await { cleanup_errors.push(format!("quiescence: {error:#}")); }
            match outcome {
                Ok(result) => data.release(result.and_then(|()| { ensure!(cleanup_errors.is_empty(), "{}", cleanup_errors.join("; ")); Ok(()) })),
                Err(panic) => { let _ = data.release::<()>(Err(anyhow::anyhow!("serial dream fixture panicked; cleanup: {}", cleanup_errors.join("; ")))); std::panic::resume_unwind(panic) }
            }
        }).await
    }

    async fn stage_reconciliation_dream(harness: &mut Harness) -> Result<Candidate> {
        let original = harness.topology.clone();
        let (topology, report) = harness.plan_dream(
            &original,
            vec![DreamProposal::Add {
                name: "Checked reconciled peer".into(),
                role: original.parts[0].role.clone(),
                instruction: "Preserve independent live work".into(),
            }],
        )?;
        let candidate = harness
            .memory
            .begin_candidate("runtime reconciliation")
            .await?;
        let keys = checked_state_keys(&harness.scope, &harness.profile)?;
        let updates = vec![
            (
                keys.membership,
                serde_json::to_value(MembershipRecord::from_topology(&topology))?,
            ),
            (
                keys.dream_undo,
                serde_json::to_value(MembershipRecord::from_topology(&original))?,
            ),
        ];
        candidate.view().put_many(&updates).await?;
        candidate
            .view()
            .append(
                &format!("{}/{}/dream-log", harness.scope, harness.config.mode),
                "dream",
                &serde_json::to_string(&report)?,
            )
            .await?;
        let target = candidate.view().revision().await?;
        let actor_namespaces =
            prepared_actor_namespaces(&harness.scope, &harness.profile, &topology)?;
        harness.pending_candidate = Some(candidate.clone());
        harness.pending_publication = Some(PendingPublication {
            config: harness.config.clone(),
            profile: harness.profile.clone(),
            actor_namespaces,
            topology,
            session: harness.session.clone(),
            updates,
            proof: PublicationProof::CandidatePromotion {
                base: candidate.base().to_owned(),
                target,
                report,
                status: CandidatePromotionStatus::Pending,
                reconciliation_attempts: 0,
            },
            scope: PublicationScope::Membership { session: false },
        });
        Ok(candidate)
    }

    #[tokio::test]
    async fn owner_loss_recovers_lost_candidate_write_without_live_publication() -> Result<()> {
        kuru_memory::test_support::closing(async {
            use futures::FutureExt as _;
            let project_directory = tempfile::tempdir()?;
            let project = project_directory.path().canonicalize()?;
            let data = kuru_memory::test_support::tempdir()?;
            let options = kuru_memory::test_support::warmed_open_options(data.path().to_owned(), crate::project_scope(&project)?).await?;
            let executable = options.supervisor.clone().context("fixture supervisor absent")?;
            let mut opened = Vec::new();
            let mut retained_harness = None;
            let outcome = std::panic::AssertUnwindSafe(tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT.saturating_mul(4), async {
                let open = || MemoryStore::open_managed_observed(options.clone(), project.clone(), executable.clone()).1;
                let memory = open().await?;
                opened.push(memory.clone());
                let sibling = open().await?;
                opened.push(sibling.clone());
                retained_harness = Some(Harness::new(config(), &project, memory.clone(), Arc::new(DemoProvider), None).await?);
                let harness = retained_harness.as_mut().context("fixture harness absent")?;
                let original_proof = memory.live_session_drivers().await?.into_iter().next().context("original claim absent")?.proof;
                let live = memory.revision().await?;
                let topology = serde_json::to_value(&harness.topology)?;
                let barrier = kuru_memory::test_support::ReplyBarrier::default();
                let selected = {
                    let candidate = harness.memory.begin_candidate("lost write owner proof").await?;
                    harness.pending_candidate = Some(candidate.clone());
                    let view = candidate.view();
                    opened.push(view.clone());
                    view.fixture_pause_next_service_reply(&barrier).await?;
                    let private_value = json!("CANDIDATE_PRIVATE_SENTINEL");
                    let mut writing = Box::pin(view.put("owner-loss-private", &private_value));
                    tokio::select! {
                        () = barrier.wait_sent() => {},
                        result = &mut writing => anyhow::bail!("candidate write replied before its pause: {result:?}"),
                    }
                    let observed = tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                        loop {
                            let status = sibling.candidate_ref_status(candidate.branch()).await?;
                            if status.head.as_deref().is_some_and(|head| head != candidate.base()) { break Ok::<_, anyhow::Error>(status); }
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                    }).await.context("owner did not accept candidate write")??;
                    drop(writing);
                    view.close_transport_for_test().await?;
                    observed
                };
                ensure!(harness.memory.put("must-stay-fenced", &json!(true)).await.is_err());
                harness.disconnect_driver_presence_for_test().await?;
                harness.memory.close_transport_for_test().await?;
                memory.close_transport_for_test().await?;
                sibling.close_transport_for_test().await?;
                kuru_memory::test_support::await_owner_release(&options).await?;
                let recovered = harness.reopen_session_after_owner_loss().await;
                let proof = harness.memory.live_session_drivers().await?.into_iter().next().context("recovered claim absent")?.proof;
                ensure!(proof.session_id == original_proof.session_id && proof.service_generation != original_proof.service_generation);
                ensure!(serde_json::to_value(&harness.topology)? == topology);
                ensure!(harness.memory.revision().await? == live);
                ensure!(harness.memory.get("owner-loss-private").await?.is_none());
                ensure!(recovered.unwrap_err().is::<CandidateResolutionRequired>());
                let candidate = harness.pending_candidate.as_ref().context("accepted candidate write was discarded")?;
                ensure!(candidate.branch() == selected.branch && Some(candidate.base()) == selected.base.as_deref());
                ensure!(Some(candidate.view().revision().await?) == selected.head);
                ensure!(candidate.view().get("owner-loss-private").await? == Some(json!("CANDIDATE_PRIVATE_SENTINEL")));
                harness.abandon_pending_dream_exact(&selected.branch, selected.base.as_deref().context("candidate base absent")?, selected.head.as_deref().context("candidate head absent")?).await?;
                ensure!(harness.pending_candidate.is_none());
                harness.reconcile().await?;
                ensure!(harness.memory.get("owner-loss-private").await?.is_none());
                ensure!(harness.memory.get("must-stay-fenced").await?.is_none());
                harness.run("continue after checked candidate recovery").await?;
                Ok::<(), anyhow::Error>(())
            })).catch_unwind().await;
            let mut cleanup = Vec::new();
            if let Some(harness) = &mut retained_harness {
                if let Err(error) = harness.shutdown(false).await { cleanup.push(format!("harness: {error:#}")); }
                opened.push(harness.memory.clone());
                if let Some(candidate) = &harness.pending_candidate { opened.push(candidate.view()); }
            }
            for memory in opened {
                if let Err(error) = memory.close().await { cleanup.push(format!("memory: {error:#}")); }
            }
            if let Err(error) = kuru_memory::test_support::await_managed_quiescence(&options).await { cleanup.push(format!("reap: {error:#}")); }
            let result = match outcome {
                Ok(result) => result.context("candidate owner recovery exceeded its bounded operation allowance")?.with_context(|| format!("candidate owner recovery cleanup: {}", cleanup.join("; "))),
                Err(panic) => { if !cleanup.is_empty() { eprintln!("candidate owner recovery cleanup: {}", cleanup.join("; ")); } std::panic::resume_unwind(panic) }
            }.and_then(|()| { ensure!(cleanup.is_empty(), "{}", cleanup.join("; ")); Ok(()) });
            data.release(result)?;
            Ok(())
        }).await
    }

    #[tokio::test]
    async fn reconciled_dream_retries_moved_promotion_and_retains_attempt_budget() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let project = tempfile::tempdir()?;
            let memory = MemoryStore::temporary().await?;
            let mut harness = Harness::new(config(), project.path(), memory.clone(), Arc::new(DemoProvider), None).await?;
            let original_count = harness.topology.parts.len();
            let lease = memory.acquire_dream_lease().await?;
            let candidate = stage_reconciliation_dream(&mut harness).await?;
            let from = candidate.view().revision().await?;
            memory.append("sibling/notes", "user", "before reconciliation").await?;
            let first_live = memory.revision().await?;
            let reconciled = candidate.reconcile_with_live(&from, &first_live).await?;
            let CandidateReconciliationResult::Reconciled { head, .. } = reconciled.result else { anyhow::bail!("runtime fixture did not merge"); };
            harness.adopt_reconciled_dream(reconciled.candidate.context("fresh handle absent")?, head)?;
            let Some(PendingPublication { proof: PublicationProof::CandidatePromotion { reconciliation_attempts, .. }, .. }) = &mut harness.pending_publication else { anyhow::bail!("proof absent"); };
            *reconciliation_attempts = 1;
            memory.put("sibling/session", &json!({"focus":"later"})).await?;
            memory.append("sibling/notes", "user", "between reconcile and promotion").await?;
            let later_live = memory.revision().await?;
            harness.drive_dream_promotion(false, &CancellationToken::new()).await?;
            let Some(PendingPublication { proof: PublicationProof::CandidatePromotion { base, target, status, reconciliation_attempts, .. }, .. }) = &harness.pending_publication else { anyhow::bail!("proof absent"); };
            ensure!(*reconciliation_attempts == 2 && *base == later_live);
            ensure!(matches!(status, CandidatePromotionStatus::Confirmed(revision) if revision == target));
            let accepted = target.clone();
            let fresh = harness.pending_candidate.as_ref().context("accepted fresh handle absent")?.clone();
            ensure!(fresh.promote_exact(&accepted).await? == accepted);
            harness.publish_pending_dream().await?;
            ensure!(harness.topology.parts.len() == original_count + 1);
            ensure!(memory.history("sibling/notes", 10).await?.len() == 2);
            ensure!(memory.get("sibling/session").await? == Some(json!({"focus":"later"})));
            drop(lease);
            harness.undo_dream().await?;
            ensure!(harness.topology.parts.iter().filter(|part| part.active).count() == original_count);
            ensure!(memory.history("sibling/notes", 10).await?.len() == 2);
            // A resumed operation cannot reset a previously consumed budget.
            let lease = memory.acquire_dream_lease().await?;
            stage_reconciliation_dream(&mut harness).await?;
            let Some(PendingPublication { proof: PublicationProof::CandidatePromotion { reconciliation_attempts, .. }, .. }) = &mut harness.pending_publication else { anyhow::bail!("proof absent"); };
            *reconciliation_attempts = 3;
            let live = memory.revision().await?;
            ensure!(harness.drive_dream_promotion(true, &CancellationToken::new()).await.unwrap_err().is::<CandidateResolutionRequired>());
            ensure!(memory.revision().await? == live);
            drop(lease);
            harness.abandon_pending_dream().await?;
            harness.shutdown(false).await?;
            memory.close().await
        }).await
    }

    #[tokio::test]
    async fn managed_reconciled_dream_recovers_lost_reply_and_adopts_new_promotion_tuple()
    -> Result<()> {
        kuru_memory::test_support::closing(async {
            use futures::FutureExt as _;
            for restart_owner in [false, true] {
            let project_directory = tempfile::tempdir()?;
            let project = project_directory.path().canonicalize()?;
            let data = kuru_memory::test_support::tempdir()?;
            let options = kuru_memory::test_support::warmed_open_options(data.path().to_owned(), crate::project_scope(&project)?).await?;
            let executable = options.supervisor.clone().context("fixture supervisor absent")?;
            let mut opened = Vec::new();
            let mut retained_harness = None;
            let outcome = std::panic::AssertUnwindSafe(async {
            let memory = MemoryStore::open_managed_observed(options.clone(), project.clone(), executable.clone()).1.await?;
            opened.push(memory.clone());
            let sibling = MemoryStore::open_managed_observed(options.clone(), project.clone(), executable.clone()).1.await?;
            opened.push(sibling.clone());
            retained_harness = Some(Harness::new(config(), &project, memory.clone(), Arc::new(DemoProvider), None).await?);
            let harness = retained_harness.as_mut().context("managed fixture harness absent")?;
            let original_count = harness.topology.parts.len();
            let lease = memory.acquire_dream_lease().await?;
            let candidate = stage_reconciliation_dream(harness).await?;
            let from = candidate.view().revision().await?;
            sibling.append("sibling/notes", "user", "newer live history").await?;
            let live = sibling.revision().await?;
            let barrier = kuru_memory::test_support::ReplyBarrier::default();
            candidate.view().fixture_pause_next_service_reply(&barrier).await?;
            let cancellation = CancellationToken::new();
            let mut operation = Box::pin(harness.drive_dream_promotion(true, &cancellation));
            tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                tokio::select! {
                    () = barrier.wait_sent() => Ok::<(), anyhow::Error>(()),
                    result = &mut operation => anyhow::bail!("reconciliation returned before paused reply: {result:?}"),
                }
            }).await.context("reconciliation frame was not sent")??;
            tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                loop {
                    let status = sibling.candidate_ref_status(candidate.branch()).await?;
                    if status.head.as_deref().is_some_and(|head| head != from) { break Ok::<(), anyhow::Error>(()); }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }).await.context("owner did not accept reconciliation")??;
            drop(operation);
            drop(lease);
            ensure!(matches!(harness.pending_publication.as_ref().map(|pending| &pending.proof),
                Some(PublicationProof::CandidatePromotion { status: CandidatePromotionStatus::Reconciling { from: retained_from, live: retained_live }, reconciliation_attempts: 1, .. }) if retained_from == &from && retained_live == &live));
            ensure!(harness.memory.put("fenced", &json!(true)).await.is_err());
            if restart_owner {
                harness.disconnect_driver_presence_for_test().await?;
                candidate.view().close_transport_for_test().await?;
                harness.memory.close_transport_for_test().await?;
                memory.close_transport_for_test().await?;
                sibling.close_transport_for_test().await?;
                kuru_memory::test_support::await_owner_release(&options).await?;
                harness.reopen_session_after_owner_loss().await?;
            }
            harness.reconcile().await?;
            ensure!(harness.topology.parts.len() == original_count + 1 && harness.pending_candidate.is_none());
            let observer = if restart_owner {
                ensure!(memory.revision().await.is_err());
                MemoryStore::open_managed_observed(options.clone(), project.clone(), executable).1.await?
            } else { sibling.clone() };
            opened.push(observer.clone());
            ensure!(observer.history("sibling/notes", 10).await?.len() == 1);
            ensure!(observer.history(&format!("{}/{}/dream-log", harness.scope, harness.config.mode), 10).await?.len() == 1);
            harness.reconcile().await?;
            Ok::<(), anyhow::Error>(())
            }).catch_unwind().await;
            let mut cleanup_errors = Vec::new();
            if let Some(harness) = &mut retained_harness {
                if let Err(error) = harness.shutdown(false).await { cleanup_errors.push(format!("shutdown: {error:#}")); }
                opened.push(harness.memory.clone());
                if let Some(candidate) = &harness.pending_candidate { opened.push(candidate.view()); }
            }
            for memory in opened {
                if let Err(error) = memory.close().await { cleanup_errors.push(format!("close: {error:#}")); }
            }
            if let Err(error) = kuru_memory::test_support::await_managed_quiescence(&options).await {
                cleanup_errors.push(format!("quiescence: {error:#}"));
            }
            let outcome = match outcome {
                Ok(result) => result.with_context(|| format!("restart_owner={restart_owner}; managed reconciliation cleanup: {}", cleanup_errors.join("; "))),
                Err(panic) => {
                    let _ = data.release::<()>(Err(anyhow::anyhow!("managed reconciliation fixture panicked; cleanup: {}", cleanup_errors.join("; "))));
                    std::panic::resume_unwind(panic)
                }
            }.and_then(|()| {
                ensure!(cleanup_errors.is_empty(), "{}", cleanup_errors.join("; "));
                Ok(())
            });
            data.release(outcome)?;
            }
            Ok(())
        }).await
    }

    #[tokio::test]
    async fn cancelled_dream_keeps_an_accepted_candidate_write_isolated() {
        kuru_memory::test_support::closing(async {
            let project = tempfile::tempdir().unwrap();
            let memory = MemoryStore::temporary().await.unwrap();
            let mut harness = Harness::new(
                config(),
                project.path(),
                memory.clone(),
                Arc::new(DemoProvider),
                None,
            )
            .await
            .unwrap();
            let live_revision = memory.revision().await.unwrap();
            let candidate = memory
                .begin_candidate("cancelled dream write")
                .await
                .unwrap();
            candidate
                .view()
                .append("candidate-proof", "dream", "accepted candidate only")
                .await
                .unwrap();
            let candidate_revision = candidate.view().revision().await.unwrap();
            assert_ne!(candidate_revision, live_revision);
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            let error = harness
                .finish_dream(
                    &candidate,
                    &harness.topology.clone(),
                    StateExpectation::Absent,
                    harness.topology.clone(),
                    &DreamReport::default(),
                    &cancellation,
                )
                .await
                .unwrap_err();
            assert!(turn_was_cancelled(&error));
            assert_eq!(memory.revision().await.unwrap(), live_revision);
            assert!(
                memory
                    .history("candidate-proof", 10)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                candidate
                    .view()
                    .history("candidate-proof", 10)
                    .await
                    .unwrap()[0]
                    .text_projection(),
                "accepted candidate only"
            );
            assert_eq!(
                candidate.view().revision().await.unwrap(),
                candidate_revision
            );
            drop(candidate);
            harness
                .run("continue after candidate cancellation")
                .await
                .unwrap();
            harness.shutdown(false).await.unwrap();
            memory.close().await.unwrap();
        })
        .await
    }

    #[tokio::test]
    async fn accepted_dream_promotion_wins_cancellation_and_publishes_exactly() {
        kuru_memory::test_support::closing(async {
            let project = tempfile::tempdir().unwrap();
            let memory = MemoryStore::temporary().await.unwrap();
            let mut harness = Harness::new(
                config(),
                project.path(),
                memory.clone(),
                Arc::new(DemoProvider),
                None,
            )
            .await
            .unwrap();
            let scope = harness.scope.clone();
            let role = harness.topology.parts[0].role.clone();
            let (topology, report) = harness
                .plan_dream(
                    &harness.topology,
                    vec![DreamProposal::Add {
                        name: "Accepted promotion".into(),
                        role,
                        instruction: "Remain durable when cancellation loses the promotion race"
                            .into(),
                    }],
                )
                .unwrap();
            let expected = serde_json::to_value(&topology).unwrap();
            let before = memory.revision().await.unwrap();
            let candidate = memory.begin_candidate("accepted promotion").await.unwrap();
            harness.pending_candidate = Some(candidate.clone());
            let original = harness.topology.clone();
            let expectation = StateExpectation::Version(
                candidate
                    .view()
                    .get_versioned(
                        &checked_state_keys(&harness.scope, &harness.profile)
                            .unwrap()
                            .membership,
                    )
                    .await
                    .unwrap()
                    .unwrap()
                    .version,
            );
            let (promoted, release) = harness.pause_after_next_memory_write();
            let mut watch = crate::progress_wait::TaskWatch::attach(&mut harness);
            let gap = crate::progress_wait::unhooked_gap_bound(&watch.hooks);
            let cancellation = CancellationToken::new();
            let controlled = cancellation.clone();
            let mut task = tokio::spawn(async move {
                let result = harness
                    .finish_dream(
                        &candidate,
                        &original,
                        expectation,
                        topology,
                        &report,
                        &controlled,
                    )
                    .await;
                (harness, candidate, result)
            });
            tokio::time::timeout(std::time::Duration::from_secs(30), promoted)
                .await
                .expect("dream promotion did not reach accepted publication")
                .unwrap();
            let accepted = memory.revision().await.unwrap();
            assert_ne!(accepted, before);
            watch
                .timings
                .mark("promotion accepted; cancelling and releasing the write");
            cancellation.cancel();
            release.send(()).unwrap();
            let (mut harness, candidate, result) = crate::progress_wait::join_on_progress(
                &mut task,
                &mut watch,
                gap,
                "accepted dream promotion",
                |(_, _, result)| crate::progress_wait::describe_result(&result),
            )
            .await;
            result.unwrap();
            assert_eq!(memory.revision().await.unwrap(), accepted);
            assert!(candidate.view().revision().await.is_err());
            // Successful publication consumed the fresh handle and proof.
            // The obsolete pre-reconciliation handle cannot replay that ref.
            assert!(harness.pending_candidate.is_none());
            assert!(candidate.promote().await.is_err());
            assert_eq!(serde_json::to_value(&harness.topology).unwrap(), expected);
            assert_eq!(
                serde_json::to_value(
                    read_topology(&memory, &scope, Mode::Freudian)
                        .await
                        .unwrap()
                )
                .unwrap(),
                expected
            );
            assert!(harness.pending_publication.is_none());
            drop(candidate);
            harness
                .run("continue after accepted promotion")
                .await
                .unwrap();
            harness.shutdown(false).await.unwrap();
            memory.close().await.unwrap();
        })
        .await
    }

    #[tokio::test]
    async fn a_recovered_candidate_write_waits_for_explicit_ref_resolution() {
        kuru_memory::test_support::closing(async {
            let project = tempfile::tempdir().unwrap();
            let memory = MemoryStore::temporary().await.unwrap();
            let mut harness = Harness::new(
                config(),
                project.path(),
                memory.clone(),
                Arc::new(DemoProvider),
                None,
            )
            .await
            .unwrap();
            let live_revision = memory.revision().await.unwrap();
            let candidate = memory
                .begin_candidate("recovered private write")
                .await
                .unwrap();
            candidate
                .view()
                .append("candidate-proof", "dream", "retained private write")
                .await
                .unwrap();
            let candidate_revision = candidate.view().revision().await.unwrap();
            harness.pending_candidate = Some(candidate);
            harness.pending_candidate_resolution = PendingCandidateResolution::Explicit;

            let error = harness.reconcile().await.unwrap_err();
            assert!(error.to_string().contains("explicit resolution"));
            assert_eq!(memory.revision().await.unwrap(), live_revision);
            assert_eq!(
                harness
                    .pending_candidate
                    .as_ref()
                    .unwrap()
                    .view()
                    .revision()
                    .await
                    .unwrap(),
                candidate_revision
            );
            harness.abandon_pending_dream().await.unwrap();
            assert!(harness.pending_candidate.is_none());
            harness
                .run("continue after explicit dream discard")
                .await
                .unwrap();
            harness.shutdown(false).await.unwrap();
            memory.close().await.unwrap();
        })
        .await
    }

    #[tokio::test]
    async fn cancelled_selected_abandon_before_dispatch_keeps_exact_ref_selectable() -> Result<()> {
        kuru_memory::test_support::closing(async {
            // One fresh store lifecycle (`MemoryStore::temporary()`, a template
            // copy's engine starts), whose single-stall term covers the owned
            // `memory.close()` under `close_budget()`, plus one statement. Every
            // other step is a statement under `QUERY_TIMEOUT`; the shutdown has no
            // MCP client, shell or hook to clean up.
            let deadline = kuru_memory::test_support::fixture_deadline(1, 0);
            tokio::time::timeout(deadline, async {
                let project = tempfile::tempdir()?;
                let memory = MemoryStore::temporary().await?;
                let mut harness = Harness::new(
                    config(),
                    project.path(),
                    memory.clone(),
                    Arc::new(DemoProvider),
                    None,
                )
                .await?;
                let live = memory.revision().await?;
                let candidate = memory.begin_candidate("selected cancellation").await?;
                candidate
                    .view()
                    .append("candidate-proof", "dream", "private before discard")
                    .await?;
                let branch = candidate.branch().to_owned();
                let base = candidate.base().to_owned();
                let head = candidate.view().revision().await?;
                harness.pending_candidate = Some(candidate);
                harness.pending_candidate_resolution = PendingCandidateResolution::Explicit;
                let (reached, release) = harness.pause_before_next_dream_abandon();
                {
                    let abandoned = harness.abandon_pending_dream_exact(&branch, &base, &head);
                    tokio::pin!(abandoned);
                    tokio::select! {
                        ready = reached => ready.context("abandon pause observer disappeared")?,
                        result = &mut abandoned => anyhow::bail!("abandon returned before dispatch pause: {result:?}"),
                    }
                }
                drop(release);
                ensure!(
                    harness.pending_candidate_resolution == PendingCandidateResolution::AbandonSent,
                    "cancelled attempt lost its pending intent"
                );
                ensure!(memory.revision().await? == live);
                let refusal = harness.reconcile().await.unwrap_err();
                ensure!(refusal.is::<CandidateResolutionRequired>());
                ensure!(
                    harness.pending_candidate_resolution == PendingCandidateResolution::Explicit,
                    "checked open ref did not restore explicit selection"
                );
                harness
                    .abandon_candidate_ref_exact(&branch, &base, &head)
                    .await?;
                ensure!(harness.pending_candidate.is_none());
                ensure!(memory.revision().await? == live);

                // A proved merge is still private until promotion. Selecting
                // discard must not run generic recovery that promotes it.
                let candidate = stage_reconciliation_dream(&mut harness).await?;
                let branch = candidate.branch().to_owned();
                let base = candidate.base().to_owned();
                let from = candidate.view().revision().await?;
                memory
                    .append("sibling/notes", "user", "retain before explicit discard")
                    .await?;
                let live = memory.revision().await?;
                let membership_key = checked_state_keys(&harness.scope, &harness.profile)?.membership;
                let membership = memory.get(&membership_key).await?;
                let Some(PendingPublication {
                    proof: PublicationProof::CandidatePromotion { status, .. },
                    ..
                }) = &mut harness.pending_publication else {
                    anyhow::bail!("staged dream proof absent");
                };
                *status = CandidatePromotionStatus::Reconciling {
                    from: from.clone(),
                    live: live.clone(),
                };
                let refusal = harness
                    .abandon_candidate_ref_exact(&branch, &base, &from)
                    .await
                    .unwrap_err();
                ensure!(refusal.is::<CandidateResolutionRequired>());
                ensure!(memory.revision().await? == live);
                ensure!(memory.get(&membership_key).await? == membership);
                ensure!(memory.candidate_ref_status(&branch).await?.head.as_deref() == Some(from.as_str()));

                let lease = memory.acquire_dream_lease().await?;
                let reconciled = candidate.reconcile_with_live(&from, &live).await?;
                let CandidateReconciliationResult::Reconciled { head, .. } = reconciled.result else {
                    anyhow::bail!("explicit discard fixture did not reconcile");
                };
                let fresh = reconciled.candidate.context("fresh reconciled handle absent")?;
                let base = fresh.base().to_owned();
                harness.adopt_reconciled_dream(fresh, head.clone())?;
                drop(lease);

                // A previously sent abandonment is a separate unresolved
                // request even when the publication proof remains Reconciled.
                harness.pending_candidate_resolution = PendingCandidateResolution::AbandonSent;
                ensure!(harness.abandon_pending_dream_exact(&branch, &base, &head).await.is_err());
                ensure!(memory.revision().await? == live);
                ensure!(memory.candidate_ref_status(&branch).await?.head.as_deref() == Some(head.as_str()));
                harness.pending_candidate_resolution = PendingCandidateResolution::Explicit;
                harness.abandon_candidate_ref_exact(&branch, &base, &head).await?;
                ensure!(harness.pending_candidate.is_none() && harness.pending_publication.is_none());
                ensure!(memory.revision().await? == live);
                ensure!(memory.get(&membership_key).await? == membership);
                let abandoned = memory.candidate_ref_status(&branch).await?;
                ensure!(abandoned.branch == branch && abandoned.state == kuru_memory::CandidateRefState::Missing);
                ensure!(memory.history("sibling/notes", 10).await?[0].plain_text() == Some("retain before explicit discard"));
                harness.shutdown(false).await?;
                memory.close().await
            })
            .await
            .with_context(|| {
                format!(
                    "selected no-send cancellation fixture exceeded fixture_deadline(1, 0) = {deadline:?}"
                )
            })??;
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn accepted_candidate_promotion_publishes_after_a_later_sibling_revision() {
        kuru_memory::test_support::closing(async {
            let project = tempfile::tempdir().unwrap();
            let memory = MemoryStore::temporary().await.unwrap();
            let mut harness = Harness::new(
                config(),
                project.path(),
                memory.clone(),
                Arc::new(DemoProvider),
                None,
            )
            .await
            .unwrap();
            let role = harness.topology.parts[0].role.clone();
            let (topology, report) = harness
                .plan_dream(
                    &harness.topology,
                    vec![DreamProposal::Add {
                        name: "Proven later".into(),
                        role,
                        instruction: "Keep the exact promoted result".into(),
                    }],
                )
                .unwrap();
            let candidate = memory.begin_candidate("settled promotion").await.unwrap();
            let base = candidate.base().to_owned();
            let actor_namespaces =
                prepared_actor_namespaces(&harness.scope, &harness.profile, &topology).unwrap();
            let updates = vec![(
                checked_state_keys(&harness.scope, &harness.profile)
                    .unwrap()
                    .membership,
                serde_json::to_value(MembershipRecord::from_topology(&topology)).unwrap(),
            )];
            candidate.view().put_many(&updates).await.unwrap();
            let target = candidate.view().revision().await.unwrap();
            let promoted = candidate.promote().await.unwrap();
            assert_eq!(promoted, target);
            memory
                .append("sibling/notes", "user", "later independent write")
                .await
                .unwrap();
            let later = memory.revision().await.unwrap();
            assert_ne!(later, target);

            // Model cancellation after the exact promotion reply but before the
            // runtime published its staged topology. A later sibling revision
            // must not invalidate that typed result or substitute value matching.
            harness.pending_candidate = Some(candidate);
            harness.pending_publication = Some(PendingPublication {
                config: harness.config.clone(),
                profile: harness.profile.clone(),
                actor_namespaces,
                topology: topology.clone(),
                session: harness.session.clone(),
                updates,
                proof: PublicationProof::CandidatePromotion {
                    base,
                    target,
                    report,
                    status: CandidatePromotionStatus::Confirmed(promoted),
                    reconciliation_attempts: 0,
                },
                scope: PublicationScope::Membership { session: false },
            });
            harness.reconcile().await.unwrap();
            assert!(harness.pending_candidate.is_none());
            assert!(harness.pending_publication.is_none());
            assert_eq!(harness.topology.parts.len(), topology.parts.len());
            assert_eq!(memory.revision().await.unwrap(), later);
            harness.shutdown(false).await.unwrap();
            memory.close().await.unwrap();
        })
        .await
    }

    #[tokio::test]
    async fn managed_lost_promotion_reply_keeps_report_and_publishes_only_typed_revision()
    -> Result<()> {
        kuru_memory::test_support::closing(async {
            use futures::FutureExt as _;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(150);
            for restart_owner in [false, true] {
                let project = tempfile::tempdir()?;
                let project_path = project.path().canonicalize()?;
                let data = kuru_memory::test_support::tempdir()?;
                // Retain fixture ownership outside the fallible body: its
                // primary error must survive cleanup and TempDir destruction.
                let mut opened = Vec::new();
                let mut retained_options = None;
                let mut retained_harness = None;
                let outcome = std::panic::AssertUnwindSafe(tokio::time::timeout_at(deadline, async {
                let options = kuru_memory::test_support::warmed_open_options(
                    data.path().to_owned(),
                    crate::project_scope(&project_path)?,
                ).await?;
                retained_options = Some(options.clone());
                let executable = options.supervisor.clone().context("fixture supervisor absent")?;
                let open = || MemoryStore::open_managed_observed(
                    options.clone(), project_path.clone(), executable.clone()
                ).1;
                let memory = open().await?;
                opened.push(memory.clone());
                let sibling = open().await?;
                opened.push(sibling.clone());
                retained_harness = Some(Harness::new(
                    config(), &project_path, memory.clone(), Arc::new(DemoProvider), None,
                ).await?);
                let harness = retained_harness.as_mut().context("fixture harness absent")?;
                let original_parts = harness.topology.parts.len();
                let role = harness.topology.parts[0].role.clone();
                if !restart_owner {
                    let live = memory.revision().await?;
                    let mismatch = memory.begin_candidate("exact target mismatch").await?;
                    mismatch.view().put("captured", &json!(1)).await?;
                    let captured = mismatch.view().revision().await?;
                    mismatch.view().put("later", &json!(2)).await?;
                    let rejection = mismatch.promote_exact(&captured).await.unwrap_err();
                    ensure!(rejection.is::<CandidateConflict>());
                    ensure!(memory.revision().await? == live);
                    mismatch.abandon().await?;
                }
                let before = memory.revision().await?;
                let barrier = harness.pause_after_next_dream_transition_frame();
                let mut dream = Box::pin(harness.apply_dream(vec![DreamProposal::Add {
                    name: "Recovered managed proposal".into(),
                    role,
                    instruction: "Publish only from typed transition proof".into(),
                }]));
            // Every step before the paused frame (reconcile, the uncontended
            // dream lease, candidate creation and writes, dream.rs:251-298) is
            // a managed reply under the client's `OPERATION_TIMEOUT`
            // (`open_managed_observed` above).
            tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                    tokio::select! {
                        () = barrier.wait_sent() => Ok::<(), anyhow::Error>(()),
                        result = &mut dream => anyhow::bail!("dream returned before paused promotion: {result:?}"),
                    }
                }).await.context("managed promotion frame was not sent")??;
                ensure!(
                    barrier.promotion_sent(),
                    "managed fixture paused a preparatory call instead of PromoteCandidate"
                );
            // Each sibling read is a managed reply under `OPERATION_TIMEOUT`;
            // the owner accepts the paused promotion inside its own statement
            // budget, below it.
            let accepted = tokio::time::timeout(kuru_memory::test_budgets::OPERATION_TIMEOUT, async {
                    loop {
                        let revision = sibling.revision().await?;
                        if revision != before {
                            break Ok::<_, anyhow::Error>(revision);
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                }).await.context("managed owner did not accept the promotion")??;
                drop(dream);
                assert_eq!(harness.topology.parts.len(), original_parts);
                let pending = harness.pending_publication.as_ref().context("staged report was lost")?;
                let PublicationProof::CandidatePromotion { target, report, .. } = &pending.proof else {
                    anyhow::bail!("managed dream lacks typed candidate proof")
                };
                assert_eq!(target, &accepted);
                assert_eq!(report.accepted.len(), 1);
                assert!(harness.pending_candidate.is_some());
                sibling.append("sibling/notes", "user", "after accepted promotion").await?;
                let later = sibling.revision().await?;
                assert_ne!(later, accepted);
                if restart_owner {
                    // The accepted reply is still lost and the old logical
                    // receipt remains. Release only generation-local transports,
                    // then reap the exact owner before querying its successor.
                    harness.disconnect_driver_presence_for_test().await?;
                    harness
                        .pending_candidate
                        .as_ref()
                        .context("lost promotion candidate was not retained")?
                        .view()
                        .close_transport_for_test()
                        .await?;
                    harness.memory.close_transport_for_test().await?;
                    memory.close_transport_for_test().await?;
                    sibling.close_transport_for_test().await?;
                    kuru_memory::test_support::await_owner_release(&options).await?;
                    harness.reopen_session_after_owner_loss().await?;
                }
                harness
                    .reconcile()
                    .await
                    .context("managed typed promotion proof was not definite on the first reconcile")?;
                assert_eq!(harness.topology.parts.len(), original_parts + 1);
                assert!(harness.pending_candidate.is_none());
                assert!(harness.pending_publication.is_none());
                if restart_owner {
                    assert!(memory.revision().await.is_err());
                }
                harness.reconcile().await?;
                let observer = if restart_owner { open().await? } else { memory.clone() };
                opened.push(observer.clone());
                assert_eq!(observer.revision().await?, later);
                let dream_log = observer
                    .history(
                        &format!("{}/{}/dream-log", harness.scope, harness.config.mode),
                        10,
                    )
                    .await?;
                assert_eq!(dream_log.len(), 1, "private candidate report was not promoted once");
                assert!(dream_log[0].text_projection().contains("Recovered managed proposal"));
                harness.reconcile().await?;
                assert_eq!(harness.topology.parts.len(), original_parts + 1);
                if restart_owner {
                    harness
                        .memory
                        .append("continuation", "user", "after checked rebind")
                        .await?;
                    assert_eq!(observer.history("continuation", 10).await?.len(), 1);
                }
                Ok::<(), anyhow::Error>(())
                })).catch_unwind().await;
                let outcome = outcome.map(|result| {
                    result.context("managed lost-promotion fixture exceeded 150 seconds")?
                });
                if let Ok(Err(error)) = &outcome {
                    eprintln!("managed lost-promotion fixture (restart_owner={restart_owner}) failed before cleanup: {error:#}");
                }
                let mut cleanup_errors = Vec::new();
                if let Some(harness) = &mut retained_harness {
                    if let Err(error) = harness.shutdown(false).await {
                        cleanup_errors.push(format!("harness shutdown: {error:#}"));
                    }
                    opened.push(harness.memory.clone());
                    if let Some(candidate) = &harness.pending_candidate {
                        opened.push(candidate.view());
                    }
                }
                for memory in opened {
                    if let Err(error) = memory.close().await {
                        cleanup_errors.push(format!("memory close: {error:#}"));
                    }
                }
                // Always await the checked owner/Dolt before releasing data,
                // even when a result, timeout or assertion ends the body early.
                if let Some(options) = &retained_options
                    && let Err(error) = kuru_memory::test_support::await_managed_quiescence(options).await
                {
                    cleanup_errors.push(format!("managed quiescence: {error:#}"));
                }
                if !cleanup_errors.is_empty() {
                    eprintln!("managed lost-promotion fixture cleanup: {}", cleanup_errors.join("; "));
                }
                match outcome {
                    Ok(result) => result.with_context(|| format!("restart_owner={restart_owner}; cleanup: {}", cleanup_errors.join("; ")) )?,
                    Err(panic) => std::panic::resume_unwind(panic),
                }
                ensure!(cleanup_errors.is_empty(), "{}", cleanup_errors.join("; "));
            }
            ensure!(
                tokio::time::Instant::now() <= deadline,
                "managed lost-promotion fixture exceeded 150 seconds including cleanup"
            );
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn cancelled_before_promotion_send_keeps_exact_open_candidate_for_resolution()
    -> Result<()> {
        kuru_memory::test_support::closing(async {
            tokio::time::timeout(std::time::Duration::from_secs(90), async {
                let project = tempfile::tempdir()?;
                let memory = MemoryStore::temporary().await?;
                let mut harness = Harness::new(
                    config(), project.path(), memory.clone(), Arc::new(DemoProvider), None,
                ).await?;
                let live = memory.revision().await?;
                let role = harness.topology.parts[0].role.clone();
                let (staged, release) = harness.pause_before_next_dream_promotion();
                let mut dream = Box::pin(harness.apply_dream(vec![DreamProposal::Add {
                    name: "Unsent promotion".into(),
                    role,
                    instruction: "Remain private until explicit resolution".into(),
                }]));
            // Reconcile, the uncontended dream lease, candidate creation and
            // the candidate's writes precede the pause (dream.rs:251-298):
            // memory statements under the budget `turn_admission_deadline`
            // follows.
            tokio::time::timeout(crate::tests::turn_admission_deadline(), async {
                    tokio::select! {
                        ready = staged => ready.context("prepromotion observer disappeared"),
                        result = &mut dream => anyhow::bail!("dream returned before prepromotion pause: {result:?}"),
                    }
                }).await.context("dream did not reach prepromotion pause")??;
                drop(dream);
                drop(release);
                let pending = harness.pending_publication.as_ref().context("staged report was lost")?;
                let PublicationProof::CandidatePromotion { report, target, .. } = &pending.proof else {
                    anyhow::bail!("cancelled dream lost candidate publication proof")
                };
                assert_eq!(report.accepted.len(), 1);
                let target = target.clone();
                assert!(harness.pending_candidate.is_some());
                assert_ne!(target, live);
                assert_eq!(memory.revision().await?, live);
                let refusal = harness.reconcile().await.unwrap_err();
                assert!(refusal.to_string().contains("explicit resolution"));
                let retained = harness.pending_candidate.as_ref().context("open ref was discarded")?;
                assert_eq!(retained.view().revision().await?, target);
                let log_key = format!("{}/{}/dream-log", harness.scope, harness.config.mode);
                assert_eq!(retained.view().history(&log_key, 10).await?.len(), 1);
                assert!(memory.history(&log_key, 10).await?.is_empty());
                harness.abandon_pending_dream().await?;
                assert!(harness.pending_candidate.is_none());
                assert_eq!(memory.revision().await?, live);
                harness.shutdown(false).await?;
                memory.close().await
            }).await.context("prepromotion cancellation fixture exceeded 90 seconds")??;
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn stale_dream_candidate_keeps_its_report_and_open_ref_until_explicit_abandon() {
        kuru_memory::test_support::closing(async {
            let project = tempfile::tempdir().unwrap();
            let data = kuru_memory::test_support::tempdir().unwrap();
            let options = kuru_memory::test_support::warmed_open_options(
                data.path().into(),
                crate::project_scope(project.path()).unwrap(),
            )
            .await
            .unwrap();
            let memory = MemoryStore::open(options).await.unwrap();
            let mut harness = Harness::new(
                config(),
                project.path(),
                memory.clone(),
                Arc::new(DemoProvider),
                None,
            )
            .await
            .unwrap();
            let original_parts = harness.topology.parts.len();
            let role = harness.topology.parts[0].role.clone();
            let membership = checked_state_keys(&harness.scope, &harness.profile)
                .unwrap()
                .membership;
            let original_membership = memory.get(&membership).await.unwrap().unwrap();
            let (staged, release) = harness.pause_before_next_dream_promotion();
            let mut watch = crate::progress_wait::TaskWatch::attach(&mut harness);
            let gap = crate::progress_wait::unhooked_gap_bound(&watch.hooks);
            let mut dream = tokio::spawn(async move {
                let result = harness
                    .apply_dream(vec![DreamProposal::Add {
                        name: "Stale proposal".into(),
                        role,
                        instruction: "Remain private after a live conflict".into(),
                    }])
                    .await;
                (harness, result)
            });
            tokio::time::timeout(std::time::Duration::from_secs(30), staged)
                .await
                .expect("dream did not stage its candidate before promotion")
                .unwrap();
            memory.put(&membership, &original_membership).await.unwrap();
            let live_revision = memory.revision().await.unwrap();
            watch
                .timings
                .mark("live advanced; releasing the staged promotion");
            release.send(()).unwrap();
            let (mut harness, result) = crate::progress_wait::join_on_progress(
                &mut dream,
                &mut watch,
                gap,
                "conflicting dream",
                |(_, result)| crate::progress_wait::describe_result(&result),
            )
            .await;
            assert!(result.unwrap_err().is::<CandidateConflict>());
            assert_eq!(harness.topology.parts.len(), original_parts);
            assert_eq!(memory.revision().await.unwrap(), live_revision);
            let pending = harness.pending_publication.as_ref().unwrap();
            let PublicationProof::CandidatePromotion { report, status, .. } = &pending.proof else {
                panic!("stale dream lost its exact candidate publication proof");
            };
            assert_eq!(report.accepted.len(), 1);
            assert!(matches!(status, CandidatePromotionStatus::OpenConflict));
            assert!(harness.pending_candidate.is_some());
            assert!(
                harness
                    .reconcile()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("conflicts")
            );
            harness.abandon_pending_dream().await.unwrap();
            assert!(harness.pending_candidate.is_none());
            assert!(harness.pending_publication.is_none());
            harness
                .run("continue after explicit conflict resolution")
                .await
                .unwrap();
            harness.shutdown(false).await.unwrap();
            memory.close().await.unwrap();
        })
        .await
    }
}
