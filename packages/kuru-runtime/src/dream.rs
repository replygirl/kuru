use anyhow::{Context, Result, ensure};
use futures::future::join_all;
use kuru_core::{
    ActorPhase, Config, Mode, Part, ToolSpec, canonical_peer_instruction,
    validate_consolidation_plan,
};
use kuru_memory::{Candidate, CandidateConflict, MemoryStore};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    Event,
    engine::{
        CancellationToken, CandidatePromotionStatus, Harness, OfferedTools,
        PendingCandidateResolution, PendingPublication, PublicationProof, Session,
        ToolHookAdmission, Topology, checked_state_keys, prepared_actor_namespaces,
        read_topology_with_profile, run_post_tool_hooks, spec, turn_was_cancelled, user,
        validate_topology_with_profile,
    },
};

#[cfg(test)]
use crate::engine::{CandidateResolutionRequired, read_topology};

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
        if reset_context {
            self.reset_context_snapshot();
        }
        cancellation.check()?;
        self.reconcile().await?;
        cancellation.check()?;
        let _dream_lease = cancellation.wait(self.memory.acquire_dream_lease()).await?;
        cancellation.check()?;
        let active = self
            .topology
            .parts
            .iter()
            .filter(|part| part.active)
            .map(|part| part.id.clone())
            .collect::<Vec<_>>();
        let plan = self.profile.memory.consolidation_plan(&active);
        validate_consolidation_plan(&plan, &active.iter().cloned().collect())?;
        self.operation_id = format!("dream-{}", Uuid::new_v4());
        let hook_host = self.hook_host();
        let hook_budget = hook_host.budget();
        let dream_tools = vec![dream_tool()];
        let candidate = self.memory.begin_candidate("dream").await?;
        // A cancelled future drops local stack state without running the
        // error path below. Keep the exact ref on Harness before the first
        // candidate write or promotion await.
        self.pending_candidate = Some(candidate.clone());
        let outcome = async {
            cancellation.check()?;
            let memory = candidate.view();
            self.emit_event(Event::Dream { actor: "pool".into(), detail: "parts are consolidating their own memories".into() });
            let ids = &plan.participants;
            let replies = join_all(ids.iter().map(|id| self.ask_in_controlled_with_invocation(&memory, id,
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
                            // The authored proposal cap and the offered dream tool are
                            // checked before any hook sees the call.
                            let admission = if index >= plan.max_proposals_per_part
                                || !dream_tools.iter().any(|tool| tool.name == original.name)
                            {
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
                                Err(anyhow::anyhow!("only dream_suggest is available"))
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
            let (topology, changes) = self.plan_dream(proposals)?;
            report.accepted = changes.accepted;
            report.rejected.extend(changes.rejected);
            self.finish_dream(&candidate, topology, &report, cancellation)
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
        let (topology, report) = self.plan_dream(proposals)?;
        let candidate = self.memory.begin_candidate("dream").await?;
        self.pending_candidate = Some(candidate.clone());
        let cancellation = CancellationToken::new();
        let outcome = async {
            self.finish_dream(&candidate, topology, &report, &cancellation)
                .await?;
            Ok(report)
        }
        .await;
        self.resolve_candidate_outcome(candidate, outcome).await
    }

    async fn finish_dream(
        &mut self,
        candidate: &Candidate,
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
            extra.push((keys.dream_undo, serde_json::to_value(&self.topology)?));
        }
        let updates = self
            .state_updates(&self.profile, &topology, &self.session, extra)
            .await?;
        cancellation.check()?;
        memory.put_many(&updates).await?;
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
            },
        });
        #[cfg(test)]
        self.pause_before_dream_promotion().await?;
        #[cfg(test)]
        if let Some(barrier) = self.dream_transition_reply_pause.take() {
            memory.fixture_pause_next_service_reply(&barrier).await?;
        }
        let promoted = match candidate.promote_exact(&target).await {
            Ok(revision) => revision,
            Err(error) if error.is::<CandidateConflict>() => {
                // The direct local store returned a definite stale-ref result.
                // Remote transport loss takes the typed outcome-query path.
                if let Some(PendingPublication {
                    proof: PublicationProof::CandidatePromotion { status, .. },
                    ..
                }) = &mut self.pending_publication
                {
                    *status = CandidatePromotionStatus::OpenConflict;
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
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
        #[cfg(test)]
        self.pause_after_memory_write().await?;
        self.publish_pending();
        Ok(())
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
                self.pending_candidate_resolution = PendingCandidateResolution::AutomaticCleanup;
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
                    | CandidatePromotionStatus::OpenConflict,
                ..
            })
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
        Ok(())
    }

    fn plan_dream(&self, proposals: Vec<DreamProposal>) -> Result<(Topology, DreamReport)> {
        ensure!(
            proposals.len() <= self.config.max_parts * 2,
            "too many dream proposals"
        );
        let mut topology = self.topology.clone();
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
        let undo = prepare_undo_dream(
            &self.config,
            &self.scope,
            &self.memory,
            Some(&self.session.id),
            Some(&self.profile),
        )
        .await?;
        self.persist_state(
            self.config.clone(),
            undo.topology,
            self.session.clone(),
            vec![(undo.key, json!(null))],
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
    let undo = prepare_undo_dream(config, scope, memory, resume, None).await?;
    memory
        .put_many(&[
            (undo.topology_key, serde_json::to_value(&undo.topology)?),
            (undo.key, json!(null)),
        ])
        .await?;
    memory.reconcile().await?;
    Ok(())
}

struct UndoDream {
    topology_key: String,
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
    let current = read_topology_with_profile(memory, scope, profile).await?;
    let keys = checked_state_keys(scope, profile)?;
    let key = keys.dream_undo;
    let value = memory
        .get(&key)
        .await?
        .filter(|value| !value.is_null())
        .context("no dreaming change to undo")?;
    let previous: Topology = serde_json::from_value(value)?;
    validate_topology_with_profile(&previous, &config, profile)?;
    let restored = restore_topology(previous, &current);
    validate_topology_with_profile(&restored, &config, profile)?;
    Ok(UndoDream {
        topology_key: keys.topology,
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

    #[tokio::test]
    async fn cancelled_dream_keeps_an_accepted_candidate_write_isolated() {
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
    }

    #[tokio::test]
    async fn accepted_dream_promotion_wins_cancellation_and_publishes_exactly() {
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
            .plan_dream(vec![DreamProposal::Add {
                name: "Accepted promotion".into(),
                role,
                instruction: "Remain durable when cancellation loses the promotion race".into(),
            }])
            .unwrap();
        let expected = serde_json::to_value(&topology).unwrap();
        let before = memory.revision().await.unwrap();
        let candidate = memory.begin_candidate("accepted promotion").await.unwrap();
        let (promoted, release) = harness.pause_after_next_memory_write();
        let cancellation = CancellationToken::new();
        let controlled = cancellation.clone();
        let task = tokio::spawn(async move {
            let result = harness
                .finish_dream(&candidate, topology, &report, &controlled)
                .await;
            (harness, candidate, result)
        });
        tokio::time::timeout(std::time::Duration::from_secs(30), promoted)
            .await
            .expect("dream promotion did not reach accepted publication")
            .unwrap();
        let accepted = memory.revision().await.unwrap();
        assert_ne!(accepted, before);
        cancellation.cancel();
        release.send(()).unwrap();
        let (mut harness, candidate, result) =
            tokio::time::timeout(std::time::Duration::from_secs(10), task)
                .await
                .expect("accepted dream promotion did not settle")
                .unwrap();
        result.unwrap();
        assert_eq!(memory.revision().await.unwrap(), accepted);
        assert!(candidate.view().revision().await.is_err());
        assert_eq!(candidate.promote().await.unwrap(), accepted);
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
    }

    #[tokio::test]
    async fn a_recovered_candidate_write_waits_for_explicit_ref_resolution() {
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
    }

    #[tokio::test]
    async fn cancelled_selected_abandon_before_dispatch_keeps_exact_ref_selectable() -> Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
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
            harness.shutdown(false).await?;
            memory.close().await
        })
        .await
        .context("selected no-send cancellation fixture exceeded 30 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn accepted_candidate_promotion_publishes_after_a_later_sibling_revision() {
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
            .plan_dream(vec![DreamProposal::Add {
                name: "Proven later".into(),
                role,
                instruction: "Keep the exact promoted result".into(),
            }])
            .unwrap();
        let candidate = memory.begin_candidate("settled promotion").await.unwrap();
        let base = candidate.base().to_owned();
        let actor_namespaces =
            prepared_actor_namespaces(&harness.scope, &harness.profile, &topology).unwrap();
        let updates = harness
            .state_updates(&harness.profile, &topology, &harness.session, vec![])
            .await
            .unwrap();
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
            },
        });
        harness.reconcile().await.unwrap();
        assert!(harness.pending_candidate.is_none());
        assert!(harness.pending_publication.is_none());
        assert_eq!(harness.topology.parts.len(), topology.parts.len());
        assert_eq!(memory.revision().await.unwrap(), later);
        harness.shutdown(false).await.unwrap();
        memory.close().await.unwrap();
    }

    #[tokio::test]
    async fn managed_lost_promotion_reply_keeps_report_and_publishes_only_typed_revision()
    -> Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(150), async {
            for restart_owner in [false, true] {
            let project = tempfile::tempdir()?;
            let project_path = project.path().canonicalize()?;
            let data = kuru_memory::test_support::tempdir()?;
            let options = kuru_memory::test_support::open_options(
                data.path().to_owned(),
                crate::project_scope(&project_path)?,
            )?;
            let executable = options.supervisor.clone().context("fixture supervisor absent")?;
            let open = || MemoryStore::open_managed_observed(
                options.clone(), project_path.clone(), executable.clone()
            ).1;
            let memory = open().await?;
            let sibling = open().await?;
            let mut harness = Harness::new(
                config(), &project_path, memory.clone(), Arc::new(DemoProvider), None,
            ).await?;
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
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                tokio::select! {
                    () = barrier.wait_sent() => Ok::<(), anyhow::Error>(()),
                    result = &mut dream => anyhow::bail!("dream returned before paused promotion: {result:?}"),
                }
            }).await.context("managed promotion frame was not sent")??;
            ensure!(
                barrier.promotion_sent(),
                "managed fixture paused a preparatory call instead of PromoteCandidate"
            );
            let accepted = tokio::time::timeout(std::time::Duration::from_secs(20), async {
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
                harness
                    .pending_candidate
                    .as_ref()
                    .context("lost promotion candidate was not retained")?
                    .view()
                    .close_transport_for_test()
                    .await?;
                memory.close_transport_for_test().await?;
                sibling.close_transport_for_test().await?;
                kuru_memory::test_support::retire_idle_service(&options).await?;
            }
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                loop {
                    match harness.reconcile().await {
                        Ok(()) => break Ok::<(), anyhow::Error>(()),
                        Err(error) if error.to_string().contains("remains uncertain") => {
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }).await.context("managed typed promotion proof did not settle")??;
            assert_eq!(harness.topology.parts.len(), original_parts + 1);
            assert!(harness.pending_candidate.is_none());
            assert!(harness.pending_publication.is_none());
            if restart_owner {
                assert!(memory.revision().await.is_err());
            }
            harness.reconcile().await?;
            let observer = if restart_owner { open().await? } else { memory.clone() };
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
            harness.shutdown(false).await?;
            harness.memory.clone().close().await?;
            observer.close().await?;
            sibling.close().await?;
            memory.close().await?;
            }
            Ok::<(), anyhow::Error>(())
        }).await.context("managed lost-promotion fixture exceeded 150 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_before_promotion_send_keeps_exact_open_candidate_for_resolution()
    -> Result<()> {
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
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
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
    }

    #[tokio::test]
    async fn stale_dream_candidate_keeps_its_report_and_open_ref_until_explicit_abandon() {
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
        let original_parts = harness.topology.parts.len();
        let role = harness.topology.parts[0].role.clone();
        let (staged, release) = harness.pause_before_next_dream_promotion();
        let dream = tokio::spawn(async move {
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
        memory
            .append("sibling/notes", "user", "advance live before promotion")
            .await
            .unwrap();
        let live_revision = memory.revision().await.unwrap();
        release.send(()).unwrap();
        let (mut harness, result) = tokio::time::timeout(std::time::Duration::from_secs(10), dream)
            .await
            .expect("conflicting dream did not settle")
            .unwrap();
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
    }
}
