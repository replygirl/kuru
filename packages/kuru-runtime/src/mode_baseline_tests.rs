//! Frozen pre-extraction behavior through the current engine loops.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::Provider;
use kuru_core::{
    Completion, CompletionRequest, Config, ContextEstimate, ContextSourceKind, Framework, Message,
    Mode, ModelInfo, RelationshipKind, ToolCall, UsagePhase,
};
use kuru_memory::MemoryStore;
use serde_json::{Value, json};

use crate::{Harness, StateReport};

#[derive(Default)]
struct Scripted {
    requests: Mutex<Vec<CompletionRequest>>,
    peer_targets: Mutex<BTreeMap<String, String>>,
}

#[derive(Default)]
struct RelationshipConsult {
    target: Mutex<String>,
    requests: Mutex<Vec<CompletionRequest>>,
}

#[async_trait]
impl Provider for RelationshipConsult {
    async fn stream(
        &self,
        request: kuru_core::CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> anyhow::Result<()> {
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            self.complete(request).await?,
        ))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let target = self.target.lock().unwrap().clone();
        let reply = if request.instructions.contains("Phase: deliberate") {
            Completion::from_legacy("draft", vec![], 1, 1)
        } else if request.instructions.contains("Phase: peer consultation") {
            assert!(
                request.tools.is_empty(),
                "consultation cannot recursively delegate"
            );
            Completion::from_legacy(
                "RELATION-ADVICE",
                vec![ToolCall {
                    id: "unaccepted-recursion".into(),
                    name: "peer_send".into(),
                    arguments: json!({"to":target,"message":"must not run"}),
                }],
                1,
                1,
            )
        } else if request
            .messages
            .iter()
            .any(|message| message.text_projection().contains("RELATION-ADVICE"))
        {
            Completion::from_legacy("consulted relationship", vec![], 1, 1)
        } else {
            Completion::from_legacy(
                "consulting",
                vec![ToolCall {
                    id: "consult-relationship".into(),
                    name: "peer_send".into(),
                    arguments: json!({"to":target,"message":"Check this answer"}),
                }],
                1,
                1,
            )
        };
        self.requests.lock().unwrap().push(request);
        Ok(reply)
    }
}

#[tokio::test]
async fn all_four_modes_allow_one_hop_speaking_consultation_with_a_relationship() {
    for mode in Mode::ALL {
        let project = tempfile::tempdir().unwrap();
        let provider = Arc::new(RelationshipConsult::default());
        let mut harness = Harness::new(
            config(mode),
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let ids = Framework::builtin(mode)
            .parts
            .into_iter()
            .map(|part| part.id)
            .collect::<Vec<_>>();
        let relation = harness
            .relate(RelationshipKind::Alliance, ids[..2].to_vec())
            .await
            .unwrap();
        *provider.target.lock().unwrap() = relation.id.clone();
        harness.focus(Some(&ids[0])).await.unwrap();
        let output = harness.run("consult relationship").await.unwrap();
        assert_eq!(output.speaker, ids[0]);
        assert_eq!(output.text, "consulted relationship");
        let peers = output
            .events
            .iter()
            .filter(|event| event.kind() == "peer")
            .collect::<Vec<_>>();
        assert_eq!(peers.len(), 1, "consultation must stay one hop");
        let rpc: Value = serde_json::from_str(&peers[0].detail()).unwrap();
        assert_eq!(
            rpc["params"]["message"]["metadata"]["recipient"],
            relation.id
        );
        let requests = provider.requests.lock().unwrap().clone();
        let consulted = requests
            .iter()
            .filter(|request| request.instructions.contains("Phase: peer consultation"))
            .collect::<Vec<_>>();
        assert_eq!(consulted.len(), 1);
        assert!(consulted[0].actor.ends_with(&relation.id));
        assert!(consulted[0].tools.is_empty());
        harness.shutdown(false).await.unwrap();
    }
}

#[async_trait]
impl Provider for Scripted {
    async fn stream(
        &self,
        request: kuru_core::CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> anyhow::Result<()> {
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            self.complete(request).await?,
        ))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let phase = if request.instructions.contains("Phase: deliberate") {
            "draft"
        } else if request.instructions.contains("Phase: speak and act") {
            "answer"
        } else {
            panic!("unexpected provider phase in mode baseline");
        };
        let identity = request.actor.rsplit('/').next().unwrap();
        let calls = if phase == "draft" {
            self.peer_targets
                .lock()
                .unwrap()
                .get(identity)
                .map(|target| {
                    vec![ToolCall {
                        id: format!("peer-{identity}"),
                        name: "peer_send".into(),
                        arguments: json!({"to":target,"message":format!("from {identity}")}),
                    }]
                })
                .unwrap_or_default()
        } else {
            vec![]
        };
        let reply = Completion::from_legacy(format!("{phase}:{identity}"), calls, 2, 3);
        self.requests.lock().unwrap().push(request);
        Ok(reply)
    }
}

#[tokio::test]
async fn all_four_modes_keep_direct_peer_routes_and_round_budget() {
    for mode in Mode::ALL {
        let project = tempfile::tempdir().unwrap();
        let provider = Arc::new(Scripted::default());
        let mut settings = config(mode);
        settings.max_rounds = 1;
        settings.max_tool_calls = 64;
        let mut harness = Harness::new(
            settings,
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let ids = Framework::builtin(mode)
            .parts
            .into_iter()
            .map(|part| part.id)
            .collect::<Vec<_>>();
        let targets = ids
            .iter()
            .enumerate()
            .map(|(index, sender)| (sender.clone(), ids[(index + 1) % ids.len()].clone()))
            .collect::<BTreeMap<_, _>>();
        *provider.peer_targets.lock().unwrap() = targets.clone();

        let output = harness.run("peer routing baseline").await.unwrap();
        assert_eq!(
            output.limit_reasons,
            Some(vec![crate::TurnLimitReason::PeerRounds])
        );
        assert!(output.limited);
        let peer_events = output
            .events
            .iter()
            .filter(|event| event.kind() == "peer")
            .collect::<Vec<_>>();
        assert_eq!(
            peer_events.len(),
            ids.len(),
            "{mode} must route one direct edge per peer"
        );
        for event in peer_events {
            let rpc: Value = serde_json::from_str(&event.detail()).unwrap();
            assert_eq!(rpc["method"], "SendMessage");
            assert_eq!(
                rpc["params"]["message"]["metadata"]["sender"],
                event.actor()
            );
            assert_eq!(
                rpc["params"]["message"]["metadata"]["recipient"],
                targets[event.actor()]
            );
        }
        let requests = provider.requests.lock().unwrap().clone();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.instructions.contains("Phase: deliberate"))
                .count(),
            ids.len()
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.instructions.contains("Phase: speak and act"))
                .count(),
            1
        );
        for (sender, recipient) in &targets {
            let private = harness.memory_for(recipient).await.unwrap();
            assert!(private.iter().any(|message| {
                message.role == "user"
                    && message
                        .text_projection()
                        .contains(&format!("from {sender}"))
            }));
        }
        harness.shutdown(false).await.unwrap();
    }
}

fn config(mode: Mode) -> Config {
    Config {
        mode,
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}

fn selection(output: &crate::TurnOutput) -> String {
    output
        .events
        .iter()
        .find(|event| event.kind() == "speaker-selection")
        .expect("speaker decision is traceable")
        .detail()
}

#[tokio::test]
async fn all_four_modes_keep_pre_extraction_requests_and_facing_outcomes() {
    for mode in Mode::ALL {
        let project = tempfile::tempdir().unwrap();
        let provider = Arc::new(Scripted::default());
        let mut harness = Harness::new(
            config(mode),
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let seeds = Framework::builtin(mode).parts;
        let ids = seeds.iter().map(|part| part.id.clone()).collect::<Vec<_>>();

        let cold = harness.run("baseline cold").await.unwrap();
        assert_eq!(cold.speaker, ids[0], "{mode} cold authored order");
        assert_eq!(selection(&cold), "mode-authored-order");
        assert_eq!(cold.text, format!("answer:{}", ids[0]));
        assert_eq!(cold.input_tokens, 2 * (ids.len() + 1) as u64);
        assert_eq!(cold.output_tokens, 3 * (ids.len() + 1) as u64);
        let requests = provider.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), ids.len() + 1);
        let deliberate = requests
            .iter()
            .filter(|request| request.instructions.contains("Phase: deliberate"))
            .collect::<Vec<_>>();
        assert_eq!(deliberate.len(), ids.len());
        assert_eq!(
            deliberate
                .iter()
                .map(|request| request.actor.rsplit('/').next().unwrap().to_owned())
                .collect::<BTreeSet<_>>(),
            ids.iter().cloned().collect()
        );
        for request in deliberate {
            assert_eq!(
                request
                    .tools
                    .iter()
                    .map(|tool| tool.name.as_str())
                    .collect::<Vec<_>>(),
                ["peer_send", "relate", "state_report", "remember"]
            );
            assert_eq!(request.current_message_count, Some(1));
            assert!(
                request
                    .messages
                    .last()
                    .unwrap()
                    .text_projection()
                    .contains("baseline cold")
            );
        }
        let speak = requests
            .iter()
            .find(|request| request.instructions.contains("Phase: speak and act"))
            .unwrap();
        assert!(speak.actor.ends_with(&ids[0]));
        let speaking_input = speak.messages.last().unwrap().text_projection();
        assert!(speaking_input.contains(&format!(
            r#""sender":"{}","text":"draft:{}""#,
            ids[0], ids[0]
        )));
        assert!(speak.tools.iter().any(|tool| tool.name == "peer_send"));

        let second = harness.run("baseline continuity").await.unwrap();
        assert_eq!(second.speaker, ids[0]);
        assert_eq!(selection(&second), "previous-completed-speaker");

        let before_target = provider.requests.lock().unwrap().len();
        let target = harness
            .run_for("baseline target", Some(&ids[1]))
            .await
            .unwrap();
        assert_eq!(target.speaker, ids[1]);
        assert_eq!(selection(&target), "caller-target");
        let targeted = {
            let requests = provider.requests.lock().unwrap();
            requests[before_target..].to_vec()
        };
        assert_eq!(
            targeted.len(),
            2,
            "target admits only one deliberator and one speaker"
        );
        assert!(
            targeted
                .iter()
                .all(|request| request.actor.ends_with(&ids[1]))
        );

        let relation = harness
            .relate(RelationshipKind::Alliance, ids[..2].to_vec())
            .await
            .unwrap();
        let related = harness.run("baseline relationship focus").await.unwrap();
        assert_eq!(related.speaker, relation.id);
        assert_eq!(selection(&related), "active-focus");
        assert_eq!(related.relationship.unwrap().members, relation.members);
        let relation_request = {
            let requests = provider.requests.lock().unwrap();
            requests
                .iter()
                .rev()
                .find(|request| request.instructions.contains("Phase: speak and act"))
                .unwrap()
                .clone()
        };
        assert!(relation_request.actor.ends_with(&relation.id));
        let input = relation_request.messages.last().unwrap().text_projection();
        for member in &relation.members {
            assert!(input.contains(&format!(r#""sender":"{member}","text":"draft:{member}""#)));
        }
        harness.focus(None).await.unwrap();
        harness.topology.states.insert(
            ids[1].clone(),
            StateReport {
                activation: 0.9,
                note: "baseline unique maximum".into(),
            },
        );
        harness.save().await.unwrap();
        let activated = harness.run("baseline activation").await.unwrap();
        assert_eq!(activated.speaker, ids[1]);
        assert_eq!(selection(&activated), "maximum-activation");
        let dream_only = BTreeMap::from([
            ("dream-z".to_string(), "candidate z".to_string()),
            ("dream-a".to_string(), "candidate a".to_string()),
        ]);
        for (id, name) in [("dream-z", "Dream Z"), ("dream-a", "Dream A")] {
            let mut part = seeds[0].clone();
            part.id = id.into();
            part.name = name.into();
            harness.topology.parts.push(part);
        }
        harness.sync_actors();
        assert_eq!(
            harness.select_speaker(&dream_only),
            ("dream-a".into(), "stable-id-order")
        );
        harness
            .topology
            .parts
            .retain(|part| !dream_only.contains_key(&part.id));
        harness.sync_actors();
        assert_eq!(
            harness.namespace(&ids[0]),
            format!("{}/{mode}/identity/{}", harness.scope, ids[0])
        );
        assert_eq!(harness.history().await.unwrap().len(), 10);
        harness.shutdown(false).await.unwrap();
    }
}

const BUILTIN_DREAM_PROMPT: &str = "Review your own history. Write a concise durable memory summary of useful facts and unresolved concerns. You may suggest a new complementary member of an existing role or retire yourself if your role is redundantly covered. A suggestion is optional; do not manufacture changes. No other tools are available during dreaming.";
const BUILTIN_DREAM_PHASE: &str =
    "dream: consolidate your own memory, optionally propose membership changes";

#[derive(Default)]
struct DreamBaseline {
    requests: Mutex<Vec<CompletionRequest>>,
}

#[async_trait]
impl Provider for DreamBaseline {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> Result<()> {
        let identity = request.actor.rsplit('/').next().unwrap().to_owned();
        let body = json!({
            "instructions": request.instructions,
            "messages": request.messages,
            "tools": request.tools,
        });
        let estimate = ContextEstimate::for_final_body(
            request.context_budget.clone().unwrap(),
            serde_json::to_vec(&body)?.len() as u64,
            false,
            vec![],
        );
        estimate.ensure_fits()?;
        sink.emit(kuru_connectors::ProviderEvent::ContextMeasured(estimate))
            .await?;
        self.requests.lock().unwrap().push(request);
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            Completion::from_legacy(format!("DREAM-SUMMARY-{identity}"), vec![], 1, 1),
        ))
        .await
    }
}

#[tokio::test]
async fn all_four_modes_keep_builtin_dream_requests_and_own_memory_isolation() {
    for mode in Mode::ALL {
        let project = tempfile::tempdir().unwrap();
        let memory = MemoryStore::temporary().await.unwrap();
        let provider = Arc::new(DreamBaseline::default());
        let mut harness = Harness::new(
            config(mode),
            project.path(),
            memory.clone(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let ids = harness
            .topology
            .parts
            .iter()
            .filter(|part| part.active)
            .map(|part| part.id.clone())
            .collect::<Vec<_>>();
        let transcript = format!("{}/transcript/{}", harness.scope, harness.session.id);
        memory
            .append_message(&transcript, &Message::text("user", "DREAM-PUBLIC-BASELINE"))
            .await
            .unwrap();
        for id in &ids {
            let namespace = harness.namespace(id);
            assert_eq!(namespace, format!("{}/{mode}/identity/{id}", harness.scope));
            memory
                .append_session_message(
                    &namespace,
                    &harness.session.id,
                    &Message::text("user", format!("OWN-HISTORY-{id}")),
                )
                .await
                .unwrap();
            memory
                .append(
                    &format!("{namespace}/notes"),
                    "note",
                    &format!("OWN-NOTE-{id}"),
                )
                .await
                .unwrap();
        }

        let report = harness.dream().await.unwrap();
        assert!(
            report.accepted.is_empty() && report.rejected.is_empty(),
            "{mode}: {report:?}"
        );
        assert_eq!(report.summaries, ids.len(), "{mode}");
        let requests = provider.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), ids.len(), "{mode}");
        assert_eq!(
            requests
                .iter()
                .map(|request| request.actor.rsplit('/').next().unwrap().to_owned())
                .collect::<BTreeSet<_>>(),
            ids.iter().cloned().collect(),
            "{mode}: every active part dreams exactly once"
        );
        for request in &requests {
            let id = request.actor.rsplit('/').next().unwrap();
            assert_eq!(
                request.actor,
                format!("{}/{mode}/identity/{id}", harness.scope)
            );
            assert!(
                request
                    .instructions
                    .contains(&format!("Phase: {BUILTIN_DREAM_PHASE}"))
            );
            assert!(request.instructions.contains("DREAM-PUBLIC-BASELINE"));
            assert!(request.instructions.contains(&format!("OWN-NOTE-{id}")));
            assert_eq!(request.current_message_count, Some(1));
            assert_eq!(
                request.messages.last().unwrap().plain_text(),
                Some(BUILTIN_DREAM_PROMPT)
            );
            assert!(
                request
                    .messages
                    .iter()
                    .any(|message| message.plain_text()
                        == Some(format!("OWN-HISTORY-{id}").as_str()))
            );
            assert_eq!(
                request
                    .tools
                    .iter()
                    .map(|tool| tool.name.as_str())
                    .collect::<Vec<_>>(),
                ["dream_suggest"]
            );
            let serialized = serde_json::to_string(request).unwrap();
            for other in &ids {
                if other != id {
                    assert!(!serialized.contains(&format!("OWN-HISTORY-{other}")));
                    assert!(!serialized.contains(&format!("OWN-NOTE-{other}")));
                }
            }
        }
        assert_eq!(
            harness.session_usage().await.unwrap().invocation_count,
            ids.len() as u64
        );
        let context = harness.subscribe_context().borrow().latest.clone().unwrap();
        assert_eq!(context.phase, UsagePhase::Dream);
        for kind in [
            ContextSourceKind::PublicTranscript,
            ContextSourceKind::PrivateHistory,
            ContextSourceKind::Notes,
        ] {
            assert_eq!(
                context
                    .runtime_sources
                    .iter()
                    .find(|source| source.kind == kind)
                    .unwrap()
                    .units,
                1
            );
        }
        for id in &ids {
            let notes = memory
                .history_window(&format!("{}/notes", harness.namespace(id)), 16)
                .await
                .unwrap();
            assert_eq!(notes.total_rows, 2);
            assert!(notes.messages.iter().any(
                |message| message.plain_text() == Some(format!("DREAM-SUMMARY-{id}").as_str())
            ));
        }
        assert_eq!(
            memory
                .history_window(&transcript, 16)
                .await
                .unwrap()
                .total_rows,
            1
        );
        assert_eq!(
            harness
                .topology
                .parts
                .iter()
                .filter(|part| part.active)
                .count(),
            ids.len()
        );
        harness.shutdown(false).await.unwrap();
        memory.close().await.unwrap();
    }
}
