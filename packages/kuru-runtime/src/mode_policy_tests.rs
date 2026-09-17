//! Test-only alternate profiles prove that the four P8 dispatch axes have effects.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{Provider, ProviderEvent, ProviderSink};
use kuru_core::{
    Completion, CompletionRequest, Config, ContentBlock, Contribution, FacingDecision, FacingInput,
    FacingPolicy, FlowPolicy, Mode, ModeProfile, ModelInfo, NativeTool, Part, PeeringPolicy,
    PermissionAction, PermissionRule, PermissionSelector, Relationship, RelationshipKind,
    RelationshipOrigin, RolesPolicy, ToolCall,
};
use kuru_memory::MemoryStore;
use serde_json::json;

use crate::{DreamProposal, Event, Harness, ToolOutcome};

#[derive(Clone, Copy)]
enum Script {
    Plain,
    DeliberatePeer,
    DeliberateRelate,
    SpeakingPeer,
    SpeakingFileWrite,
}

struct ScriptedProvider {
    script: Script,
    recipient: String,
    requests: Mutex<Vec<CompletionRequest>>,
    speaking_call_issued: AtomicBool,
}

impl ScriptedProvider {
    fn new(script: Script, recipient: String) -> Self {
        Self {
            script,
            recipient,
            requests: Mutex::new(vec![]),
            speaking_call_issued: AtomicBool::new(false),
        }
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let deliberate = request.instructions.contains("Phase: deliberate");
        let speak = request.instructions.contains("Phase: speak and act");
        let calls = match self.script {
            Script::DeliberatePeer if deliberate => vec![ToolCall {
                id: "direct-edge".into(),
                name: "peer_send".into(),
                arguments: json!({"to":self.recipient,"message":"policy edge"}),
            }],
            Script::DeliberateRelate if deliberate => vec![ToolCall {
                id: "proposed-relation".into(),
                name: "relate".into(),
                arguments: json!({
                    "kind":"alliance",
                    "members":[request.actor.rsplit('/').next().unwrap(), self.recipient],
                }),
            }],
            Script::SpeakingPeer
                if speak && !self.speaking_call_issued.swap(true, Ordering::SeqCst) =>
            {
                vec![ToolCall {
                    id: "speaking-consultation".into(),
                    name: "peer_send".into(),
                    arguments: json!({"to":self.recipient,"message":"consult once"}),
                }]
            }
            Script::SpeakingFileWrite
                if speak && !self.speaking_call_issued.swap(true, Ordering::SeqCst) =>
            {
                vec![ToolCall {
                    id: "alternate-profile-file".into(),
                    name: "file_write".into(),
                    arguments: json!({"path":"alternate-policy-must-not-write.txt","content":"not authorized"}),
                }]
            }
            _ => vec![],
        };
        self.requests.lock().unwrap().push(request.clone());
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            format!(
                "{}:{}",
                if deliberate { "draft" } else { "answer" },
                request.actor
            ),
            calls,
            1,
            1,
        )))
        .await
    }
}

fn config() -> Config {
    Config {
        mode: Mode::Ifs,
        provider: "demo".into(),
        model: "demo".into(),
        max_rounds: 2,
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}

struct AlternateRoles(Vec<Part>);

impl RolesPolicy for AlternateRoles {
    fn seeds(&self) -> Vec<Part> {
        self.0.clone()
    }
}

struct SelectivePeering {
    base: Arc<dyn PeeringPolicy>,
    deny_direct: bool,
}

struct SubstituteRelationship {
    members: Vec<String>,
}

impl PeeringPolicy for SubstituteRelationship {
    fn allows_direct(&self, sender: &str, recipient: &str, active: &BTreeSet<String>) -> bool {
        sender != recipient && active.contains(sender) && active.contains(recipient)
    }

    fn relationship(
        &self,
        _origin: &RelationshipOrigin,
        kind: RelationshipKind,
        _members: Vec<String>,
        _active_parts: &BTreeSet<String>,
    ) -> Result<Relationship> {
        Relationship::new(kind, self.members.clone())
    }
}

impl PeeringPolicy for SelectivePeering {
    fn allows_direct(&self, sender: &str, recipient: &str, active: &BTreeSet<String>) -> bool {
        !self.deny_direct && self.base.allows_direct(sender, recipient, active)
    }

    fn relationship(
        &self,
        origin: &RelationshipOrigin,
        kind: RelationshipKind,
        members: Vec<String>,
        active_parts: &BTreeSet<String>,
    ) -> Result<Relationship> {
        self.base.relationship(origin, kind, members, active_parts)
    }
}

#[derive(Clone, Copy)]
enum FlowChange {
    FirstSecond,
    RedirectTarget,
    InactiveFirst,
    NonpendingNext,
    PausePending,
    NoConsultation,
    ForgedContribution,
    OmitContributions,
}

struct AlternateFlow {
    base: Arc<dyn FlowPolicy>,
    change: FlowChange,
}

impl FlowPolicy for AlternateFlow {
    fn initial_recipients(&self, target: Option<&str>, active_parts: &[String]) -> Vec<String> {
        match self.change {
            FlowChange::FirstSecond if target.is_none() => vec![active_parts[1].clone()],
            FlowChange::RedirectTarget if target.is_some() => vec![
                active_parts
                    .iter()
                    .find(|id| Some(id.as_str()) != target)
                    .unwrap()
                    .clone(),
            ],
            FlowChange::InactiveFirst if target.is_none() => vec!["inactive-part".into()],
            _ => self.base.initial_recipients(target, active_parts),
        }
    }

    fn next_recipients(&self, pending: &BTreeSet<String>) -> Vec<String> {
        match self.change {
            FlowChange::NonpendingNext if !pending.is_empty() => vec!["not-pending".into()],
            FlowChange::PausePending => vec![],
            _ => self.base.next_recipients(pending),
        }
    }

    fn consultation_recipients(
        &self,
        recipient: &str,
        live_identities: &BTreeSet<String>,
    ) -> Vec<String> {
        match self.change {
            FlowChange::NoConsultation => vec![],
            _ => self
                .base
                .consultation_recipients(recipient, live_identities),
        }
    }

    fn shared_contributions(
        &self,
        speaker: &str,
        relationship: Option<&Relationship>,
        drafts: &BTreeMap<String, String>,
    ) -> Vec<Contribution> {
        match self.change {
            FlowChange::ForgedContribution => vec![Contribution {
                sender: speaker.into(),
                text: "forged contribution".into(),
            }],
            FlowChange::OmitContributions => vec![],
            _ => self
                .base
                .shared_contributions(speaker, relationship, drafts),
        }
    }
}

#[derive(Clone, Copy)]
enum FacingChange {
    SecondDraft,
    Ineligible,
    RedirectTarget,
}

struct AlternateFacing {
    base: Arc<dyn FacingPolicy>,
    change: FacingChange,
}

impl FacingPolicy for AlternateFacing {
    fn choose(&self, input: FacingInput<'_>) -> Result<FacingDecision> {
        match self.change {
            FacingChange::SecondDraft if input.target.is_none() => Ok(FacingDecision {
                speaker: input.drafts.iter().nth(1).unwrap().clone(),
                reason: "alternate-eligible-draft",
            }),
            FacingChange::Ineligible => Ok(FacingDecision {
                speaker: "inactive-speaker".into(),
                reason: "invalid-test-choice",
            }),
            FacingChange::RedirectTarget if input.target.is_some() => Ok(FacingDecision {
                speaker: input
                    .live
                    .iter()
                    .find(|id| Some(id.as_str()) != input.target)
                    .unwrap()
                    .clone(),
                reason: "redirected-target",
            }),
            _ => self.base.choose(input),
        }
    }
}

#[tokio::test]
async fn alternate_roles_seed_fresh_topology_and_preserve_required_role_on_reopen() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let mut seeds = profile.roles.seeds();
    seeds[0].id = "alternate-required-seed".into();
    seeds[0].name = "Alternate Required".into();
    profile.roles = Arc::new(AlternateRoles(seeds.clone()));
    let provider = Arc::new(ScriptedProvider::new(Script::Plain, String::new()));
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile.clone(),
    )
    .await
    .unwrap();
    assert_eq!(harness.topology.parts, seeds);
    let session = harness.session.id.clone();
    let report = harness
        .apply_dream(vec![DreamProposal::Retire {
            id: "alternate-required-seed".into(),
        }])
        .await
        .unwrap();
    assert!(report.accepted.is_empty());
    assert_eq!(report.rejected.len(), 1);
    assert!(harness.topology.parts[0].active);
    harness.shutdown(false).await.unwrap();
    let mut resumed = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider,
        Some(&session),
        profile,
    )
    .await
    .unwrap();
    assert_eq!(resumed.topology.parts, seeds);
    let added = resumed
        .apply_dream(vec![DreamProposal::Add {
            name: "Alternate Extra".into(),
            role: seeds[0].role.clone(),
            instruction: "Help without taking authority".into(),
        }])
        .await
        .unwrap();
    assert_eq!(added.accepted.len(), 1);
    let added_id = resumed
        .topology
        .parts
        .iter()
        .find(|part| part.name == "Alternate Extra")
        .unwrap()
        .id
        .clone();
    resumed.run("later history survives undo").await.unwrap();
    let history = resumed.history().await.unwrap();
    resumed.undo_dream().await.unwrap();
    assert_eq!(resumed.history().await.unwrap(), history);
    assert!(resumed.topology.parts[0].active);
    assert!(
        !resumed
            .topology
            .parts
            .iter()
            .find(|part| part.id == added_id)
            .unwrap()
            .active
    );
    resumed.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn alternate_flow_changes_actual_first_provider_and_valid_facing_changes_speaker() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let seeds = profile.roles.seeds();
    profile.flow = Arc::new(AlternateFlow {
        base: profile.flow.clone(),
        change: FlowChange::FirstSecond,
    });
    let provider = Arc::new(ScriptedProvider::new(Script::Plain, String::new()));
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    let output = harness.run("only second deliberates").await.unwrap();
    assert_eq!(output.speaker, seeds[1].id);
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.actor.ends_with(&seeds[1].id))
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();

    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    profile.facing = Arc::new(AlternateFacing {
        base: profile.facing.clone(),
        change: FacingChange::SecondDraft,
    });
    let provider = Arc::new(ScriptedProvider::new(Script::Plain, String::new()));
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    let output = harness.run("alternate speaker").await.unwrap();
    assert_eq!(output.speaker, seeds[1].id);
    assert_eq!(
        output
            .events
            .iter()
            .find(|event| event.kind() == "speaker-selection")
            .unwrap()
            .detail(),
        "alternate-eligible-draft"
    );
    assert!(
        provider
            .requests()
            .last()
            .unwrap()
            .actor
            .ends_with(&seeds[1].id)
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn flow_may_pause_next_round_and_omit_drafts_without_losing_pending_mail() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let ids = profile.roles.authored_order();
    profile.flow = Arc::new(AlternateFlow {
        base: profile.flow.clone(),
        change: FlowChange::PausePending,
    });
    let provider = Arc::new(ScriptedProvider::new(
        Script::DeliberatePeer,
        ids[1].clone(),
    ));
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    let output = harness
        .run_for("defer peer reply", Some(&ids[0]))
        .await
        .unwrap();
    assert!(output.limited);
    assert!(output.events.iter().any(|event| event.kind() == "peer"));
    assert!(
        provider
            .requests()
            .iter()
            .all(|request| !request.actor.ends_with(&ids[1]))
    );
    assert!(
        harness
            .memory_for(&ids[1])
            .await
            .unwrap()
            .iter()
            .any(|message| {
                message.role == "user" && message.text_projection().contains("policy edge")
            })
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();

    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    profile.flow = Arc::new(AlternateFlow {
        base: profile.flow.clone(),
        change: FlowChange::OmitContributions,
    });
    let provider = Arc::new(ScriptedProvider::new(Script::Plain, String::new()));
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    harness
        .run("only explicit user material in speaking input")
        .await
        .unwrap();
    let requests = provider.requests();
    assert!(
        requests
            .iter()
            .any(|request| request.instructions.contains("Phase: deliberate"))
    );
    let speaking = requests
        .iter()
        .find(|request| request.instructions.contains("Phase: speak and act"))
        .unwrap();
    assert!(
        speaking
            .messages
            .last()
            .unwrap()
            .text_projection()
            .contains("Explicit contributions to this speaking identity: []")
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn denied_direct_edge_and_suppressed_consultation_have_no_delivery() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let ids = profile.roles.authored_order();
    profile.peering = Arc::new(SelectivePeering {
        base: profile.peering.clone(),
        deny_direct: true,
    });
    let provider = Arc::new(ScriptedProvider::new(
        Script::DeliberatePeer,
        ids[1].clone(),
    ));
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    let output = harness.run_for("denied edge", Some(&ids[0])).await.unwrap();
    assert!(output.events.iter().all(|event| event.kind() != "peer"));
    assert!(
        provider
            .requests()
            .iter()
            .all(|request| !request.actor.ends_with(&ids[1]))
    );
    assert!(harness.memory_for(&ids[1]).await.unwrap().is_empty());
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();

    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    profile.flow = Arc::new(AlternateFlow {
        base: profile.flow.clone(),
        change: FlowChange::NoConsultation,
    });
    let provider = Arc::new(ScriptedProvider::new(Script::SpeakingPeer, ids[1].clone()));
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    let output = harness
        .run_for("suppress consultation", Some(&ids[0]))
        .await
        .unwrap();
    assert!(output.events.iter().all(|event| event.kind() != "peer"));
    assert!(
        provider
            .requests()
            .iter()
            .all(|request| !request.actor.ends_with(&ids[1]))
    );
    assert!(harness.memory_for(&ids[1]).await.unwrap().is_empty());
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn policy_cannot_substitute_an_active_canonical_relationship_excluding_its_proposer() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let ids = profile.roles.authored_order();
    profile.peering = Arc::new(SubstituteRelationship {
        members: vec![ids[1].clone(), ids[2].clone()],
    });
    let provider = Arc::new(ScriptedProvider::new(
        Script::DeliberateRelate,
        ids[1].clone(),
    ));
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    let output = harness
        .run_for("reject substituted relationship", Some(&ids[0]))
        .await
        .unwrap();
    assert!(
        output
            .events
            .iter()
            .all(|event| event.kind() != "relationship")
    );
    assert!(output.events.iter().all(|event| event.kind() != "peer"));
    assert!(harness.topology.relationships.is_empty());
    assert!(
        provider.requests().iter().all(|request| {
            !request.actor.ends_with(&ids[1]) && !request.actor.ends_with(&ids[2])
        })
    );
    let proposer_history = harness.memory_for(&ids[0]).await.unwrap();
    assert!(proposer_history.iter().any(|message| {
        message.role == "tool"
            && message
                .text_projection()
                .contains("mode changed a relationship proposal")
    }));
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn alternate_facing_still_refuses_a_denied_file_tool_before_any_effect() {
    let project = tempfile::tempdir().unwrap();
    let marker = project.path().join("alternate-policy-must-not-write.txt");
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    profile.facing = Arc::new(AlternateFacing {
        base: profile.facing.clone(),
        change: FacingChange::SecondDraft,
    });
    let provider = Arc::new(ScriptedProvider::new(
        Script::SpeakingFileWrite,
        String::new(),
    ));
    let mut settings = config();
    settings.permissions.push(PermissionRule {
        action: PermissionAction::Deny,
        selector: PermissionSelector::native(NativeTool::FileWrite),
        path: None,
    });
    let mut harness = Harness::new_with_test_profile(
        settings,
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    let output = harness.run("ask alternate speaker to write").await.unwrap();
    let settled = output
        .events
        .iter()
        .filter_map(|event| match event {
            Event::ToolSettled { observation, .. } if observation.name == "file_write" => {
                Some(observation.outcome)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(settled, [ToolOutcome::Denied]);
    assert!(!marker.exists());
    let requests = provider.requests();
    let speaking = requests
        .iter()
        .filter(|request| request.instructions.contains("Phase: speak and act"))
        .collect::<Vec<_>>();
    assert_eq!(speaking.len(), 2);
    assert!(
        speaking
            .iter()
            .all(|request| request.actor.ends_with(&output.speaker))
    );
    assert!(
        speaking[1]
            .messages
            .iter()
            .flat_map(|message| &message.blocks)
            .any(|block| {
                matches!(block, ContentBlock::ToolResult { call_id, is_error: true, .. }
            if call_id == "alternate-profile-file")
            })
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn invalid_flow_and_facing_outputs_stop_before_unrelated_provider_dispatch() {
    for flow_change in [
        FlowChange::RedirectTarget,
        FlowChange::InactiveFirst,
        FlowChange::NonpendingNext,
        FlowChange::ForgedContribution,
    ] {
        let project = tempfile::tempdir().unwrap();
        let memory = MemoryStore::temporary().await.unwrap();
        let mut profile = ModeProfile::builtin(Mode::Ifs);
        let ids = profile.roles.authored_order();
        profile.flow = Arc::new(AlternateFlow {
            base: profile.flow.clone(),
            change: flow_change,
        });
        let script = if matches!(flow_change, FlowChange::NonpendingNext) {
            Script::DeliberatePeer
        } else {
            Script::Plain
        };
        let provider = Arc::new(ScriptedProvider::new(script, ids[1].clone()));
        let mut harness = Harness::new_with_test_profile(
            config(),
            project.path(),
            memory.clone(),
            provider.clone(),
            None,
            profile,
        )
        .await
        .unwrap();
        let before = harness.topology.clone();
        let result = if matches!(
            flow_change,
            FlowChange::InactiveFirst | FlowChange::ForgedContribution
        ) {
            harness.run("invalid policy output").await
        } else {
            harness
                .run_for("invalid policy output", Some(&ids[0]))
                .await
        };
        assert!(result.is_err(), "invalid Flow result was accepted");
        assert_eq!(harness.topology.parts, before.parts);
        let requests = provider.requests();
        if matches!(flow_change, FlowChange::ForgedContribution) {
            assert!(
                requests
                    .iter()
                    .all(|request| { !request.instructions.contains("Phase: speak and act") })
            );
        } else {
            assert!(
                requests
                    .iter()
                    .all(|request| !request.actor.ends_with(&ids[1]))
            );
        }
        harness.shutdown(false).await.unwrap();
        memory.close().await.unwrap();
    }
    for facing_change in [FacingChange::Ineligible, FacingChange::RedirectTarget] {
        let project = tempfile::tempdir().unwrap();
        let memory = MemoryStore::temporary().await.unwrap();
        let mut profile = ModeProfile::builtin(Mode::Ifs);
        let ids = profile.roles.authored_order();
        profile.facing = Arc::new(AlternateFacing {
            base: profile.facing.clone(),
            change: facing_change,
        });
        let provider = Arc::new(ScriptedProvider::new(Script::Plain, String::new()));
        let mut harness = Harness::new_with_test_profile(
            config(),
            project.path(),
            memory.clone(),
            provider.clone(),
            None,
            profile,
        )
        .await
        .unwrap();
        let before = harness.topology.clone();
        let result = if matches!(facing_change, FacingChange::Ineligible) {
            harness.run("invalid speaker").await
        } else {
            harness.run_for("redirect target", Some(&ids[0])).await
        };
        assert!(result.is_err(), "invalid Facing result was accepted");
        assert_eq!(harness.topology.parts, before.parts);
        assert!(
            provider
                .requests()
                .iter()
                .all(|request| !request.instructions.contains("Phase: speak and act"))
        );
        harness.shutdown(false).await.unwrap();
        memory.close().await.unwrap();
    }
}
