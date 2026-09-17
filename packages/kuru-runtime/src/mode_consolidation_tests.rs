//! A bounded live proof that dream proposal application follows the selected
//! plan order while provider requests remain concurrent.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{Provider, ProviderEvent, ProviderSink};
use kuru_core::{
    Completion, CompletionRequest, Config, ConsolidationPlan, ContextEstimate, MemoryPolicy, Mode,
    ModeProfile, ModelInfo, StateKeys, ToolCall, UsagePhase,
};
use kuru_memory::MemoryStore;
use serde_json::json;

use crate::{DreamProposal, Harness};

struct OrderedOneProposal;

impl MemoryPolicy for OrderedOneProposal {
    fn identity_namespace(&self, scope: &str, mode: Mode, identity: &str) -> String {
        format!("{scope}/{mode}/identity/{identity}")
    }

    fn transcript_namespace(&self, scope: &str, session: &str) -> String {
        format!("{scope}/transcript/{session}")
    }

    fn state_keys(&self, scope: &str, mode: Mode) -> StateKeys {
        StateKeys {
            topology: format!("{scope}/{mode}/topology"),
            dream_undo: format!("{scope}/{mode}/dream-undo"),
        }
    }

    fn consolidation_plan(&self, active: &[String]) -> ConsolidationPlan {
        ConsolidationPlan {
            participants: vec![active[1].clone(), active[0].clone()],
            prompt: "submit your one ordered membership proposal".into(),
            phase: "dream: ordered one-proposal fixture".into(),
            max_proposals_per_part: 1,
        }
    }
}

struct CompetingProposals {
    first: String,
    role: String,
    requests: Mutex<Vec<String>>,
}

#[async_trait]
impl Provider for CompetingProposals {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let request_bytes = serde_json::to_vec(&request)?.len() as u64;
        sink.emit(ProviderEvent::ContextMeasured(
            ContextEstimate::for_final_body(
                request.context_budget.clone().unwrap(),
                request_bytes,
                false,
                vec![],
            ),
        ))
        .await?;
        let identity = request.actor.rsplit('/').next().unwrap().to_owned();
        self.requests.lock().unwrap().push(identity.clone());
        let first = identity == self.first;
        let name = if first {
            "First planned addition"
        } else {
            "Second planned addition"
        };
        let mut calls = vec![ToolCall {
            id: if first {
                "first-allowed"
            } else {
                "second-allowed"
            }
            .into(),
            name: "dream_suggest".into(),
            arguments: json!({
                "action": "add",
                "name": name,
                "role": self.role,
                "instruction": "Preserve a useful distinct perspective.",
            }),
        }];
        if first {
            calls.push(ToolCall {
                id: "first-excess".into(),
                name: "dream_suggest".into(),
                arguments: json!({
                    "action": "add",
                    "name": "Excess first proposal",
                    "role": self.role,
                    "instruction": "This proposal must be refused.",
                }),
            });
        }
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "summary", calls, 3, 2,
        )))
        .await
    }
}

#[tokio::test]
async fn dream_applies_ordered_subset_and_one_proposal_limit_with_typed_usage() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let seeds = profile.roles.seeds();
    let first = seeds[1].id.clone();
    let second = seeds[0].id.clone();
    let provider = Arc::new(CompetingProposals {
        first: first.clone(),
        role: seeds[0].role.clone(),
        requests: Mutex::new(vec![]),
    });
    profile.memory = Arc::new(OrderedOneProposal);
    let mut harness = Harness::new_with_test_profile(
        Config {
            mode: Mode::Ifs,
            provider: "demo".into(),
            model: "demo".into(),
            max_parts: seeds.len() + 1,
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();

    let context = harness.subscribe_context();
    let report = harness.dream().await.unwrap();
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert!(
        matches!(&report.accepted[..], [DreamProposal::Add { name, .. }] if name == "First planned addition")
    );
    assert!(
        report
            .rejected
            .iter()
            .any(|reason| reason.contains("at most 1 dream proposals"))
    );
    assert!(
        report
            .rejected
            .iter()
            .any(|reason| reason.contains("Second planned addition"))
    );
    assert!(
        harness
            .topology
            .parts
            .iter()
            .any(|part| part.name == "First planned addition" && part.active)
    );
    assert!(
        !harness
            .topology
            .parts
            .iter()
            .any(|part| part.name == "Second planned addition")
    );
    let first_receipts = memory
        .history(&harness.checked_namespace(&first).unwrap(), 20)
        .await
        .unwrap();
    assert!(first_receipts.iter().any(|message| {
        message.role == "tool"
            && message.text_projection().contains("first-excess")
            && message
                .text_projection()
                .contains("at most 1 dream proposals")
    }));
    assert_eq!(harness.session_usage().await.unwrap().invocation_count, 2);
    assert_eq!(
        context.borrow().latest.as_ref().unwrap().phase,
        UsagePhase::Dream
    );
    assert_eq!(
        provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [first, second].into()
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}
