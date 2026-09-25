//! Real-Dolt proof that selected visibility changes provider material and no authority.

use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{DemoProvider, Provider, ProviderEvent, ProviderSink};
use kuru_core::{
    ActorPhase, Completion, CompletionRequest, Config, ContentBlock, ContextEstimate,
    ContextSource, ContextSourceKind, Message, Mode, ModeProfile, ModelInfo, RelationshipKind,
    ToolCall, VisibilityPolicy,
};
use kuru_memory::{
    ContextSummaryCheckpoint, ContextSummaryRecord, MemoryStore, ReasoningSummaryRecord,
};
use serde_json::json;

use crate::{Harness, RequestContext};

fn config() -> Config {
    Config {
        mode: Mode::Ifs,
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}

#[derive(Clone)]
enum BadRelationPlan {
    CrossPrivate(String),
    MissingExplicitInput,
}

struct SelectedVisibility {
    base: Arc<dyn VisibilityPolicy>,
    omit_public: bool,
    omit_notes: bool,
    omit_history: bool,
    bad_relation: Option<(String, BadRelationPlan)>,
    deny_delivery: bool,
}

impl SelectedVisibility {
    fn new(base: Arc<dyn VisibilityPolicy>) -> Self {
        Self {
            base,
            omit_public: false,
            omit_notes: false,
            omit_history: false,
            bad_relation: None,
            deny_delivery: false,
        }
    }
}

impl VisibilityPolicy for SelectedVisibility {
    fn context_sources(&self, identity: &str, phase: ActorPhase) -> Vec<ContextSource> {
        if let Some((relation, plan)) = &self.bad_relation
            && relation == identity
        {
            return match plan {
                BadRelationPlan::CrossPrivate(member) => vec![
                    ContextSource::OwnHistory(member.clone()),
                    ContextSource::ExplicitInput,
                ],
                BadRelationPlan::MissingExplicitInput => {
                    vec![ContextSource::OwnHistory(identity.into())]
                }
            };
        }
        let mut selected = self.base.context_sources(identity, phase);
        selected.retain(|source| match source {
            ContextSource::PublicTranscript => !self.omit_public,
            ContextSource::OwnNotes(_) => !self.omit_notes,
            ContextSource::OwnHistory(_) => !self.omit_history,
            ContextSource::ExplicitInput => true,
        });
        selected
    }

    fn allows_delivery(&self, sender: &str, recipient: &str, active: &BTreeSet<String>) -> bool {
        !self.deny_delivery && self.base.allows_delivery(sender, recipient, active)
    }
}

struct MeasuredRequest {
    request: CompletionRequest,
    context: ContextEstimate,
}

#[derive(Default)]
struct RecordingDemo {
    requests: Mutex<Vec<MeasuredRequest>>,
}

struct MeasurementSink<'a> {
    downstream: &'a mut dyn ProviderSink,
    measured: &'a mut Option<ContextEstimate>,
}

impl ProviderSink for MeasurementSink<'_> {
    fn emit<'a>(
        &'a mut self,
        event: ProviderEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let ProviderEvent::ContextMeasured(context) = &event {
                *self.measured = Some(context.clone());
            }
            self.downstream.emit(event).await
        })
    }
}

#[async_trait]
impl Provider for RecordingDemo {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        DemoProvider.models().await
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let mut measured = None;
        let result = DemoProvider
            .stream(
                request.clone(),
                &mut MeasurementSink {
                    downstream: sink,
                    measured: &mut measured,
                },
            )
            .await;
        self.requests.lock().unwrap().push(MeasuredRequest {
            request,
            context: measured.expect("Demo measured its effective serialized request"),
        });
        result
    }
}

struct ObservedSelection {
    request: CompletionRequest,
    measured: ContextEstimate,
    attempts: usize,
    context: RequestContext,
    history_rows: u64,
    public_rows: u64,
    note_rows: u64,
}

async fn observe_selection(
    omit_public: bool,
    omit_notes: bool,
    omit_history: bool,
    window: Option<u64>,
) -> ObservedSelection {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let mut visibility = SelectedVisibility::new(profile.visibility.clone());
    visibility.omit_public = omit_public;
    visibility.omit_notes = omit_notes;
    visibility.omit_history = omit_history;
    profile.visibility = Arc::new(visibility);
    let provider = Arc::new(RecordingDemo::default());
    let mut settings = config();
    settings.assumed_context_window_tokens = window;
    if window.is_some() {
        settings.context_output_reserve_tokens = Some(200);
    }
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
    let actor = harness.topology.parts[0].id.clone();
    let history_key = harness.namespace(&actor);
    let notes_key = format!("{history_key}/notes");
    let transcript_key = format!("{}/transcript/{}", harness.scope, harness.session.id);
    memory
        .append_session_message(
            &history_key,
            &harness.session.id,
            &Message::text("user", "HISTORY-SENTINEL-🪶".repeat(100)),
        )
        .await
        .unwrap();
    memory
        .append_message(
            &transcript_key,
            &Message::text("user", "PUBLIC-SENTINEL-🌊".repeat(100)),
        )
        .await
        .unwrap();
    memory
        .append(&notes_key, "note", &"NOTE-SENTINEL-é".repeat(100))
        .await
        .unwrap();
    let inputs = vec![
        Message::text("user", "CURRENT-INPUT-🦉"),
        Message::tool_result("current-call", json!({"value":"CURRENT-RECEIPT-🦉"}), false),
    ];
    harness
        .ask(
            &actor,
            inputs.clone(),
            "speak and act: selected visibility",
            vec![],
        )
        .await
        .unwrap();
    let (attempts, request, measured) = {
        let observed = provider.requests.lock().unwrap();
        assert!(!observed.is_empty());
        assert!(
            observed[..observed.len() - 1]
                .iter()
                .all(|attempt| attempt.context.ensure_fits().is_err())
        );
        assert!(observed.last().unwrap().context.ensure_fits().is_ok());
        (
            observed.len(),
            observed.last().unwrap().request.clone(),
            observed.last().unwrap().context.clone(),
        )
    };
    assert_eq!(request.current_message_count, Some(inputs.len()));
    assert_eq!(
        request.messages[request.messages.len() - inputs.len()..],
        inputs
    );
    assert!(request.messages.iter().any(|message| {
        matches!(message.blocks.as_slice(), [ContentBlock::ToolResult { call_id, .. }] if call_id == "current-call")
    }));
    let context = harness.subscribe_context().borrow().latest.clone().unwrap();
    let history_rows = memory
        .history_window(&history_key, 100)
        .await
        .unwrap()
        .total_rows;
    let public_rows = memory
        .history_window(&transcript_key, 100)
        .await
        .unwrap()
        .total_rows;
    let note_rows = memory
        .history_window(&notes_key, 100)
        .await
        .unwrap()
        .total_rows;
    assert_eq!(harness.session_usage().await.unwrap().invocation_count, 1);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
    ObservedSelection {
        request,
        measured,
        attempts,
        context,
        history_rows,
        public_rows,
        note_rows,
    }
}

fn source_units(context: &RequestContext, kind: ContextSourceKind) -> u64 {
    context
        .runtime_sources
        .iter()
        .find(|source| source.kind == kind)
        .unwrap()
        .units
}

#[tokio::test]
async fn selected_ambient_sources_change_real_demo_body_and_fit_without_erasing_history() {
    let full = observe_selection(false, false, false, None).await;
    let public_hidden = observe_selection(true, false, false, None).await;
    let notes_hidden = observe_selection(false, true, false, None).await;
    let hidden = observe_selection(true, true, true, None).await;
    let full_body = serde_json::to_string(&full.request).unwrap();
    let hidden_body = serde_json::to_string(&hidden.request).unwrap();
    for sentinel in ["PUBLIC-SENTINEL", "NOTE-SENTINEL", "HISTORY-SENTINEL"] {
        assert!(full_body.contains(sentinel));
        assert!(!hidden_body.contains(sentinel));
    }
    for sentinel in ["CURRENT-INPUT", "CURRENT-RECEIPT"] {
        assert!(full_body.contains(sentinel));
        assert!(hidden_body.contains(sentinel));
    }
    assert!(hidden.measured.final_body_bytes < full.measured.final_body_bytes);
    assert!(hidden.measured.estimated_input_tokens < full.measured.estimated_input_tokens);
    assert!(public_hidden.measured.estimated_input_tokens < full.measured.estimated_input_tokens);
    assert!(notes_hidden.measured.estimated_input_tokens < full.measured.estimated_input_tokens);
    assert_eq!(
        source_units(&public_hidden.context, ContextSourceKind::PublicTranscript),
        0
    );
    assert_eq!(
        source_units(&public_hidden.context, ContextSourceKind::Notes),
        1
    );
    assert_eq!(
        source_units(&public_hidden.context, ContextSourceKind::PrivateHistory),
        1
    );
    assert_eq!(
        source_units(&notes_hidden.context, ContextSourceKind::PublicTranscript),
        1
    );
    assert_eq!(
        source_units(&notes_hidden.context, ContextSourceKind::Notes),
        0
    );
    assert_eq!(
        source_units(&notes_hidden.context, ContextSourceKind::PrivateHistory),
        1
    );
    assert!(
        !serde_json::to_string(&public_hidden.request)
            .unwrap()
            .contains("PUBLIC-SENTINEL")
    );
    assert!(
        !serde_json::to_string(&notes_hidden.request)
            .unwrap()
            .contains("NOTE-SENTINEL")
    );
    assert!(full.measured.ensure_fits().is_ok() && hidden.measured.ensure_fits().is_ok());
    for kind in [
        ContextSourceKind::PrivateHistory,
        ContextSourceKind::PublicTranscript,
        ContextSourceKind::Notes,
    ] {
        assert_eq!(source_units(&full.context, kind), 1);
        assert_eq!(source_units(&hidden.context, kind), 0);
    }
    assert_eq!(
        source_units(&hidden.context, ContextSourceKind::CurrentInput),
        1
    );
    assert_eq!(
        source_units(&hidden.context, ContextSourceKind::RequiredReceipts),
        1
    );
    assert_eq!(hidden.context.omitted_private_rows, 0);
    assert_eq!(hidden.context.omitted_public_rows, 0);
    assert_eq!(hidden.context.omitted_note_rows, 0);
    assert_eq!(
        (full.history_rows, full.public_rows, full.note_rows),
        (4, 1, 1)
    );
    assert_eq!(
        (hidden.history_rows, hidden.public_rows, hidden.note_rows),
        (4, 1, 1)
    );
}

#[tokio::test]
async fn selected_sources_change_real_fit_retry_under_the_same_reserved_window() {
    let hidden_unbounded = observe_selection(true, true, true, None).await;
    let window = hidden_unbounded.measured.estimated_input_tokens + 300;
    let full = observe_selection(false, false, false, Some(window)).await;
    let hidden = observe_selection(true, true, true, Some(window)).await;
    assert!(
        full.attempts > 1,
        "selected ambient rows should exceed the bounded window"
    );
    assert_eq!(
        hidden.attempts, 1,
        "omitted ambient rows should fit immediately"
    );
    assert_eq!(full.measured.budget.window.value, window);
    assert_eq!(hidden.measured.budget.window.value, window);
    assert_eq!(full.measured.budget.output_reserve_tokens, 200);
    assert_eq!(hidden.measured.budget.output_reserve_tokens, 200);
    assert!(
        full.context.omitted_private_rows
            + full.context.omitted_public_rows
            + full.context.omitted_note_rows
            > 0
    );
    assert_eq!(
        (
            hidden.context.omitted_private_rows,
            hidden.context.omitted_public_rows,
            hidden.context.omitted_note_rows
        ),
        (0, 0, 0)
    );
    for observed in [full, hidden] {
        assert!(
            observed
                .request
                .messages
                .iter()
                .any(|message| message.text_projection().contains("CURRENT-INPUT"))
        );
        assert!(observed.request.messages.iter().flat_map(|message| &message.blocks).any(|block| {
            matches!(block, ContentBlock::ToolResult { call_id, .. } if call_id == "current-call")
        }));
        assert_eq!(
            (
                observed.history_rows,
                observed.public_rows,
                observed.note_rows
            ),
            (4, 1, 1)
        );
    }
}

#[tokio::test]
async fn relationship_request_never_reads_member_private_context() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(RecordingDemo::default());
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        ModeProfile::builtin(Mode::Ifs),
    )
    .await
    .unwrap();
    let ids = harness
        .topology
        .parts
        .iter()
        .take(2)
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    for id in &ids {
        memory
            .append_message(
                &harness.namespace(id),
                &Message::text("user", format!("MEMBER-PRIVATE-{id}")),
            )
            .await
            .unwrap();
        memory
            .append(
                &format!("{}/notes", harness.namespace(id)),
                "note",
                &format!("MEMBER-NOTE-{id}"),
            )
            .await
            .unwrap();
    }
    let relation = harness
        .relate(RelationshipKind::Alliance, ids.clone())
        .await
        .unwrap();
    harness
        .run_for("relationship has only its own context", Some(&relation.id))
        .await
        .unwrap();
    {
        let requests = provider.requests.lock().unwrap();
        let related = requests
            .iter()
            .filter(|item| item.request.actor.ends_with(&relation.id))
            .collect::<Vec<_>>();
        assert!(!related.is_empty());
        for item in related {
            let body = serde_json::to_string(&item.request).unwrap();
            assert!(!body.contains("MEMBER-PRIVATE"));
            assert!(!body.contains("MEMBER-NOTE"));
        }
    }
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn missing_input_and_cross_private_relation_plan_refuse_before_provider_or_admission() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(RecordingDemo::default());
    let mut original = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let ids = original
        .topology
        .parts
        .iter()
        .take(2)
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    let relation = original
        .relate(RelationshipKind::Alliance, ids.clone())
        .await
        .unwrap();
    let session = original.session.id.clone();
    original.shutdown(false).await.unwrap();

    for bad_plan in [
        BadRelationPlan::CrossPrivate(ids[0].clone()),
        BadRelationPlan::MissingExplicitInput,
    ] {
        let mut profile = ModeProfile::builtin(Mode::Ifs);
        let mut visibility = SelectedVisibility::new(profile.visibility.clone());
        visibility.bad_relation = Some((relation.id.clone(), bad_plan));
        profile.visibility = Arc::new(visibility);
        let mut harness = Harness::new_with_test_profile(
            config(),
            project.path(),
            memory.clone(),
            provider.clone(),
            Some(&session),
            profile,
        )
        .await
        .unwrap();
        let result = harness
            .ask(
                &relation.id,
                vec![Message::text("user", "must not be dispatched")],
                "speak and act: forbidden relation source",
                vec![],
            )
            .await;
        assert!(result.is_err());
        assert!(provider.requests.lock().unwrap().is_empty());
        assert_eq!(harness.session_usage().await.unwrap().invocation_count, 0);
        assert!(harness.memory_for(&relation.id).await.unwrap().is_empty());
        harness.shutdown(false).await.unwrap();
    }
    memory.close().await.unwrap();
}

struct PeerSender {
    recipient: String,
    requests: Mutex<Vec<CompletionRequest>>,
}

#[async_trait]
impl Provider for PeerSender {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let calls = if request.instructions.contains("Phase: deliberate")
            && self.requests.lock().unwrap().is_empty()
        {
            vec![ToolCall {
                id: "visibility-denied-edge".into(),
                name: "peer_send".into(),
                arguments: json!({"to":self.recipient,"message":"must not deliver"}),
            }]
        } else {
            vec![]
        };
        self.requests.lock().unwrap().push(request);
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "reply", calls, 1, 1,
        )))
        .await
    }
}

#[tokio::test]
async fn visibility_delivery_veto_has_truthful_refusal_and_no_mail_or_recipient_call() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let ids = profile.roles.authored_order();
    let mut visibility = SelectedVisibility::new(profile.visibility.clone());
    visibility.deny_delivery = true;
    profile.visibility = Arc::new(visibility);
    let provider = Arc::new(PeerSender {
        recipient: ids[1].clone(),
        requests: Mutex::new(vec![]),
    });
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
        .run_for("peer delivery veto", Some(&ids[0]))
        .await
        .unwrap();
    assert!(output.events.iter().all(|event| event.kind() != "peer"));
    assert!(
        provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| !request.actor.ends_with(&ids[1]))
    );
    assert!(harness.memory_for(&ids[1]).await.unwrap().is_empty());
    assert!(
        harness
            .memory_for(&ids[0])
            .await
            .unwrap()
            .iter()
            .any(|message| {
                message.role == "tool" && message.text_projection().contains("visibility")
            })
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn own_history_admits_only_typed_same_actor_cross_session_summaries() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let profile = ModeProfile::builtin(Mode::Ifs);
    let provider = Arc::new(RecordingDemo::default());
    let mut settings = config();
    settings.assumed_context_window_tokens = Some(4_096);
    settings.context_output_reserve_tokens = Some(256);
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
    let actor = harness.topology.parts[0].id.clone();
    let other_actor = harness.topology.parts[1].id.clone();
    let namespace = harness.namespace(&actor);
    let other_namespace = harness.namespace(&other_actor);
    let current_session = harness.session.id.clone();

    async fn seed_summary(
        memory: &MemoryStore,
        namespace: &str,
        session: &str,
        actor: &str,
        raw: &str,
        summary: &str,
        private_reasoning: &str,
    ) {
        memory
            .append_session_message(namespace, session, &Message::text("user", raw))
            .await
            .unwrap();
        let snapshot = memory
            .session_source_snapshot(namespace, session, namespace, 0, 1_024)
            .await
            .unwrap();
        let through = snapshot.through_inclusive.unwrap();
        let operation_id = format!("cross-session-operation-{session}-{actor}");
        let invocation_id = format!("cross-session-invocation-{session}-{actor}");
        memory
            .checkpoint_context_summary(&ContextSummaryCheckpoint {
                record: ContextSummaryRecord {
                    actor_namespace: namespace.into(),
                    session_id: session.into(),
                    source_namespace: namespace.into(),
                    summary_namespace: crate::context_compaction::summary_namespace(namespace),
                    source_view: snapshot.view,
                    source_revision: snapshot.revision,
                    after_sequence: 0,
                    through_sequence: through,
                    turn_id: None,
                    operation_id: Some(operation_id.clone()),
                    producer_actor_id: Some(actor.into()),
                    invocation_id: invocation_id.clone(),
                    summary: summary.into(),
                },
                private_reasoning: vec![ReasoningSummaryRecord {
                    session_id: session.into(),
                    turn_id: None,
                    operation_id: Some(operation_id),
                    actor_id: actor.into(),
                    invocation_id,
                    item_id: Some("private-item".into()),
                    output_index: Some(0),
                    summary_index: 0,
                    text: private_reasoning.into(),
                }],
            })
            .await
            .unwrap();
    }

    seed_summary(
        &memory,
        &namespace,
        "older-foreign-session",
        &actor,
        "OLDER_FOREIGN_RAW_MUST_NOT_APPEAR",
        &"OLDER_SHARED_SUMMARY_MUST_BE_OMITTED".repeat(1_000),
        "OLDER_FOREIGN_REASONING_MUST_NOT_APPEAR",
    )
    .await;
    seed_summary(
        &memory,
        &namespace,
        "foreign-session",
        &actor,
        "FOREIGN_RAW_MUST_NOT_APPEAR",
        "ALLOWED_SHARED_SUMMARY",
        "FOREIGN_REASONING_MUST_NOT_APPEAR",
    )
    .await;
    seed_summary(
        &memory,
        &other_namespace,
        "other-actor-session",
        &other_actor,
        "OTHER_ACTOR_RAW_MUST_NOT_APPEAR",
        "OTHER_ACTOR_SUMMARY_MUST_NOT_APPEAR",
        "OTHER_ACTOR_REASONING_MUST_NOT_APPEAR",
    )
    .await;
    seed_summary(
        &memory,
        &namespace,
        &current_session,
        &actor,
        "CURRENT_RAW_BEFORE_CURSOR_MUST_NOT_APPEAR",
        "CURRENT_SESSION_SUMMARY_ONCE",
        "CURRENT_REASONING_MUST_NOT_APPEAR",
    )
    .await;
    let current_cursor = memory
        .context_summary_cursor(&namespace, &current_session, &namespace)
        .await
        .unwrap()
        .unwrap();

    harness
        .ask(
            &actor,
            vec![Message::text("user", "current-session-input")],
            "speak and act: shared continuity",
            vec![],
        )
        .await
        .unwrap();
    let allowed = provider
        .requests
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .request
        .clone();
    let allowed_text = serde_json::to_string(&allowed.messages).unwrap();
    assert!(allowed_text.contains("ALLOWED_SHARED_SUMMARY"));
    assert!(!allowed_text.contains("OLDER_SHARED_SUMMARY_MUST_BE_OMITTED"));
    assert_eq!(
        allowed_text.matches("CURRENT_SESSION_SUMMARY_ONCE").count(),
        1
    );
    for forbidden in [
        "FOREIGN_RAW_MUST_NOT_APPEAR",
        "FOREIGN_REASONING_MUST_NOT_APPEAR",
        "OLDER_FOREIGN_RAW_MUST_NOT_APPEAR",
        "OLDER_FOREIGN_REASONING_MUST_NOT_APPEAR",
        "CURRENT_RAW_BEFORE_CURSOR_MUST_NOT_APPEAR",
        "CURRENT_REASONING_MUST_NOT_APPEAR",
        "OTHER_ACTOR_RAW_MUST_NOT_APPEAR",
        "OTHER_ACTOR_SUMMARY_MUST_NOT_APPEAR",
        "OTHER_ACTOR_REASONING_MUST_NOT_APPEAR",
    ] {
        assert!(!allowed_text.contains(forbidden), "leaked {forbidden}");
    }
    let allowed_context = harness.subscribe_context().borrow().latest.clone().unwrap();
    assert_eq!(allowed_context.omitted_summary_rows, 1);
    assert!(allowed_context.runtime_sources.iter().any(|source| {
        source.kind == ContextSourceKind::ContextSummary && !source.mandatory && source.units == 1
    }));
    assert_eq!(
        memory
            .context_summary_cursor(&namespace, &current_session, &namespace)
            .await
            .unwrap()
            .unwrap(),
        current_cursor,
        "foreign continuity advanced the current session cursor"
    );

    let mut denied = SelectedVisibility::new(harness.profile.visibility.clone());
    denied.omit_history = true;
    harness.profile.visibility = Arc::new(denied);
    harness
        .ask(
            &actor,
            vec![Message::text("user", "denied-current-input")],
            "speak and act: denied shared continuity",
            vec![],
        )
        .await
        .unwrap();
    let denied = provider
        .requests
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .request
        .clone();
    let denied_text = serde_json::to_string(&denied.messages).unwrap();
    assert!(!denied_text.contains("ALLOWED_SHARED_SUMMARY"));
    assert!(denied_text.contains("denied-current-input"));
    assert_eq!(
        memory
            .context_summary_cursor(&namespace, &current_session, &namespace)
            .await
            .unwrap()
            .unwrap(),
        current_cursor
    );

    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}
