use anyhow::{Context, Result, ensure};
use futures::future::join_all;
use kuru_core::{Config, Mode, Part, ToolSpec};
use kuru_memory::{Candidate, MemoryStore};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::engine::{
    Harness, PendingPublication, Session, Topology, read_topology, spec, user, validate_topology,
};

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
        self.reconcile().await?;
        let candidate = self.memory.begin_candidate("dream").await?;
        let memory = candidate.view();
        self.emit(
            "dream",
            "pool",
            "parts are consolidating their own memories",
        );
        let ids = self
            .topology
            .parts
            .iter()
            .filter(|p| p.active)
            .map(|p| p.id.clone())
            .collect::<Vec<_>>();
        let replies = join_all(ids.iter().map(|id| self.ask_in(&memory, id,
            vec![user("Review your own history. Write a concise durable memory summary of useful facts and unresolved concerns. You may suggest a new complementary member of an existing role or retire yourself if your role is redundantly covered. A suggestion is optional; do not manufacture changes. No other tools are available during dreaming.")],
            "dream: consolidate your own memory, optionally propose membership changes", vec![dream_tool()]))).await;
        let mut report = DreamReport::default();
        let mut proposals = vec![];
        for (id, reply) in ids.into_iter().zip(replies) {
            match reply {
                Err(error) if error.is::<crate::actor::MemoryFailure>() => return Err(error),
                Err(error) => report.rejected.push(format!("{id}: {error:#}")),
                Ok(reply) => {
                    if !reply.text.trim().is_empty() {
                        memory
                            .append(
                                &format!("{}/notes", self.namespace(&id)),
                                "dream",
                                &crate::actor::truncate_text(&reply.text, 8192),
                            )
                            .await?;
                        report.summaries += 1;
                    }
                    for (index, call) in reply.calls.into_iter().enumerate() {
                        let result = if index >= 2 {
                            Err(anyhow::anyhow!(
                                "at most two dream proposals are accepted per part"
                            ))
                        } else if call.name != "dream_suggest" {
                            Err(anyhow::anyhow!("only dream_suggest is available"))
                        } else {
                            serde_json::from_value::<DreamProposal>(call.arguments.clone())
                                .map_err(Into::into)
                        };
                        let outcome = match result {
                            Ok(proposal) => {
                                if matches!(&proposal, DreamProposal::Retire { id: target } if target != &id)
                                {
                                    let error = format!("{id}: parts may only retire themselves");
                                    report.rejected.push(error.clone());
                                    error
                                } else {
                                    proposals.push(proposal);
                                    "proposal submitted for validation".to_string()
                                }
                            }
                            Err(error) => {
                                let error = format!("{id}: {error}");
                                report.rejected.push(error.clone());
                                error
                            }
                        };
                        memory
                            .append(
                                &self.namespace(&id),
                                "tool",
                                &json!({"call_id":call.id,"output":outcome}).to_string(),
                            )
                            .await?;
                    }
                }
            }
        }
        let (topology, changes) = self.plan_dream(proposals)?;
        report.accepted = changes.accepted;
        report.rejected.extend(changes.rejected);
        self.finish_dream(&candidate, topology, &report).await?;
        self.emit(
            "dream",
            "pool",
            format!(
                "{} summaries, {} changes, {} rejected proposals",
                report.summaries,
                report.accepted.len(),
                report.rejected.len()
            ),
        );
        Ok(report)
    }

    pub async fn apply_dream(&mut self, proposals: Vec<DreamProposal>) -> Result<DreamReport> {
        self.reconcile().await?;
        let (topology, report) = self.plan_dream(proposals)?;
        let candidate = self.memory.begin_candidate("dream").await?;
        self.finish_dream(&candidate, topology, &report).await?;
        Ok(report)
    }

    async fn finish_dream(
        &mut self,
        candidate: &Candidate,
        topology: Topology,
        report: &DreamReport,
    ) -> Result<()> {
        let memory = candidate.view();
        let mut extra = vec![(
            format!("{}/{}/last-dream", self.scope, self.config.mode),
            json!({"id":Uuid::new_v4(), "base":candidate.base()}),
        )];
        if !report.accepted.is_empty() {
            extra.push((
                format!("{}/{}/dream-undo", self.scope, self.config.mode),
                serde_json::to_value(&self.topology)?,
            ));
        }
        let updates = self
            .state_updates(&memory, &topology, &self.session, extra)
            .await?;
        memory.put_many(&updates).await?;
        memory
            .append(
                &format!("{}/{}/dream-log", self.scope, self.config.mode),
                "dream",
                &serde_json::to_string(report)?,
            )
            .await?;
        self.pending_publication = Some(PendingPublication {
            config: self.config.clone(),
            topology,
            session: self.session.clone(),
            updates,
        });
        candidate.promote().await?;
        self.publish_pending();
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
                    if name.trim().is_empty()
                        || name.len() > 128
                        || instruction.trim().is_empty()
                        || instruction.len() > 8192
                    {
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
                        candidate.parts.push(Part {
                            id: Uuid::new_v4().to_string(),
                            name: name.clone(),
                            role: role.clone(),
                            instruction: instruction.clone(),
                            active: true,
                        });
                        Ok(())
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
            .and_then(|()| validate_topology(&candidate, &self.config));
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
    let undo = prepare_undo_dream(config, scope, memory, resume).await?;
    memory
        .put_many(&[
            (
                format!("{scope}/{}/topology", undo.mode),
                serde_json::to_value(&undo.topology)?,
            ),
            (undo.key, json!(null)),
        ])
        .await?;
    memory.reconcile().await?;
    Ok(())
}

struct UndoDream {
    mode: Mode,
    topology: Topology,
    key: String,
}

async fn prepare_undo_dream(
    config: &Config,
    scope: &str,
    memory: &MemoryStore,
    resume: Option<&str>,
) -> Result<UndoDream> {
    config.validate()?;
    memory.reconcile().await?;
    let mode = resume_mode(config, scope, memory, resume).await?;
    let mut config = config.clone();
    config.mode = mode;
    config.validate()?;
    let current = read_topology(memory, scope, mode).await?;
    let key = format!("{scope}/{mode}/dream-undo");
    let value = memory
        .get(&key)
        .await?
        .filter(|value| !value.is_null())
        .context("no dreaming change to undo")?;
    let previous: Topology = serde_json::from_value(value)?;
    validate_topology(&previous, &config)?;
    let restored = restore_topology(previous, &current);
    validate_topology(&restored, &config)?;
    Ok(UndoDream {
        mode,
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
