//! Pins the per-operation contract and the memory service wire surface.

use super::*;
use kuru_core::{Usage, UsagePhase};
use serde_json::json;

const HANDLE: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
const GENERATION: &str = "01234567-89ab-cdef-0123-456789abcdef";

fn message() -> Message {
    Message::text("user", "hello")
}

fn values() -> Vec<(String, Value)> {
    vec![("key".into(), Value::String("value".into()))]
}

fn reasoning_summary() -> crate::ReasoningSummaryRecord {
    crate::ReasoningSummaryRecord {
        session_id: "session".into(),
        turn_id: Some("turn".into()),
        operation_id: Some("operation".into()),
        actor_id: "actor".into(),
        invocation_id: "invocation".into(),
        item_id: Some("item".into()),
        output_index: Some(0),
        summary_index: 0,
        text: "summary".into(),
    }
}

fn invocation_start() -> kuru_core::InvocationStart {
    kuru_core::InvocationStart {
        session_id: "session".into(),
        invocation_id: "invocation".into(),
        operation_id: "operation".into(),
        phase: UsagePhase::Speak,
        actor_id: "actor".into(),
        route: "responses".into(),
        model: "model".into(),
        price_at_invocation: None,
    }
}

/// One sample per `ViewOperation` variant, every optional field populated so
/// the wire pin sees each field name.
pub(super) fn view_operation_samples() -> Result<Vec<ViewOperation>> {
    Ok(vec![
        ViewOperation::Append {
            namespace: "actor".into(),
            role: "user".into(),
            content: "hello".into(),
        },
        ViewOperation::AppendMessage {
            namespace: "actor".into(),
            message: message(),
        },
        ViewOperation::AppendSessionMessage {
            namespace: "actor".into(),
            session_id: "session".into(),
            message: message(),
        },
        ViewOperation::Checkpoint {
            namespace: "actor".into(),
            messages: vec![message()],
            values: values(),
        },
        ViewOperation::CheckpointSession {
            namespace: "transcript".into(),
            session_id: "session".into(),
            messages: vec![message()],
            values: values(),
            public_turn: Some(crate::SessionTurnCheckpoint::Admit {
                expected_generation: 0,
                turn_id: "turn".into(),
                label: Some("label".into()),
                expected_transcript_rows: Some(0),
            }),
            mode: Some(crate::SessionModeCheckpoint {
                expected_generation: 0,
                expected_mode: Mode::Ifs,
                mode: Mode::Jungian,
            }),
        },
        ViewOperation::History {
            namespace: "actor".into(),
            limit: 1,
        },
        ViewOperation::HistoryWindow {
            namespace: "actor".into(),
            limit: 1,
        },
        ViewOperation::SessionHistoryWindow {
            namespace: "actor".into(),
            session_id: "session".into(),
            limit: 1,
        },
        ViewOperation::SessionHistoryWindowAfter {
            namespace: "actor".into(),
            session_id: "session".into(),
            after_exclusive: 0,
            limit: 1,
        },
        ViewOperation::SessionCatalogPage {
            lifecycle_state: Some(crate::SessionLifecycleState::Active),
            cursor: Some(crate::SessionCatalogCursor {
                updated_order: 1,
                session_id: "session".into(),
            }),
            expected_revision: Some("revision".into()),
            limit: 1,
        },
        ViewOperation::SessionCatalogRecord {
            session_id: "session".into(),
        },
        ViewOperation::CreateSession {
            session_id: "session".into(),
            mode: Mode::Ifs,
            label: "label".into(),
        },
        ViewOperation::RenameSession {
            session_id: "session".into(),
            expected_generation: 0,
            label: "label".into(),
        },
        ViewOperation::RemoveSession {
            session_id: "session".into(),
            expected_generation: 0,
        },
        ViewOperation::RestoreSession {
            session_id: "session".into(),
            expected_generation: 0,
        },
        ViewOperation::ForkSession {
            source_session_id: "source".into(),
            expected_source_generation: 0,
            source_node_id: "node".into(),
            child_session_id: "child".into(),
            label: "label".into(),
        },
        ViewOperation::PublicTranscriptPage {
            session_id: "session".into(),
            cursor: Some(crate::PublicTranscriptCursor {
                session_id: "session".into(),
                revision: "revision".into(),
                head_node_id: Some("node".into()),
                next: crate::store::PublicTranscriptPosition::Turn {
                    node_id: "node".into(),
                },
            }),
            limit: 1,
        },
        ViewOperation::SessionSourceSnapshot {
            actor_namespace: "actor".into(),
            session_id: "session".into(),
            source_namespace: "source".into(),
            after_exclusive: 0,
            limit: 1,
        },
        ViewOperation::CheckpointContextSummary {
            record: crate::ContextSummaryRecord {
                actor_namespace: "actor".into(),
                session_id: "session".into(),
                source_namespace: "source".into(),
                summary_namespace: "summary".into(),
                source_view: "main".into(),
                source_revision: "revision".into(),
                after_sequence: 0,
                through_sequence: 1,
                turn_id: Some("turn".into()),
                operation_id: Some("operation".into()),
                producer_actor_id: Some("actor".into()),
                invocation_id: "invocation".into(),
                summary: "summary".into(),
            },
            private_reasoning: vec![reasoning_summary()],
        },
        ViewOperation::ContextSummaryCursor {
            actor_namespace: "actor".into(),
            session_id: "session".into(),
            source_namespace: "source".into(),
        },
        ViewOperation::ContextSummaryWindow {
            actor_namespace: "actor".into(),
            summary_namespace: "summary".into(),
            session_id: Some("session".into()),
            source_namespace: Some("source".into()),
            limit: 1,
        },
        ViewOperation::Notes {
            namespace: "actor".into(),
            limit: 1,
        },
        ViewOperation::ForgetNote {
            namespace: "actor".into(),
            sequence: 1,
        },
        ViewOperation::PutMany { values: values() },
        ViewOperation::Get { key: "key".into() },
        ViewOperation::Clear {
            namespace: "actor".into(),
        },
        ViewOperation::Reconcile,
        ViewOperation::Revision,
        ViewOperation::Revisions { limit: 1 },
        ViewOperation::Status,
    ])
}

/// One sample per `LedgerOperation` variant.
pub(super) fn ledger_operation_samples() -> Vec<LedgerOperation> {
    vec![
        LedgerOperation::MarkNewSession {
            session_id: "session".into(),
        },
        LedgerOperation::Admit {
            start: Box::new(invocation_start()),
        },
        LedgerOperation::Observe {
            invocation_id: "invocation".into(),
            observation: UsageObservation {
                sequence: 1,
                terminal: true,
                usage: Usage {
                    input_tokens: Some(1),
                    output_tokens: Some(1),
                    cached_input_tokens: Some(0),
                    reasoning_output_tokens: Some(0),
                },
            },
        },
        LedgerOperation::Settle {
            invocation_id: "invocation".into(),
            outcome: InvocationOutcome::Succeeded,
        },
        LedgerOperation::Session {
            session_id: "session".into(),
        },
    ]
}

/// One sample per `ServiceCall` variant. `View` and `Ledger` carry one
/// representative operation; their operations are sampled separately.
pub(super) fn service_call_samples() -> Result<Vec<ServiceCall>> {
    let export_cursor: ExportCursor =
        serde_json::from_value(json!({"snapshot": HANDLE, "phase": {"Messages": 1}}))?;
    Ok(vec![
        ServiceCall::RetireIfIdle,
        ServiceCall::TryAcquireDreamLease,
        ServiceCall::AppendMessage {
            namespace: "actor".into(),
            message: message(),
        },
        ServiceCall::HistoryWindow {
            namespace: "actor".into(),
            limit: 1,
        },
        ServiceCall::Notes {
            namespace: "actor".into(),
            limit: 1,
        },
        ServiceCall::PutMany { values: values() },
        ServiceCall::PutReasoningSummaries {
            records: vec![reasoning_summary()],
        },
        ServiceCall::Get { key: "key".into() },
        ServiceCall::Reconcile,
        ServiceCall::Revision,
        ServiceCall::Outcome {
            original_id: HANDLE,
            original_generation: GENERATION.into(),
            view: "main".into(),
            method: "view.append".into(),
            argument_digest: "digest".into(),
        },
        ServiceCall::View {
            candidate: Some(HANDLE),
            operation: Box::new(ViewOperation::Status),
        },
        ServiceCall::BeginCandidate {
            label: "dream".into(),
        },
        ServiceCall::CandidateOutcome {
            original_id: HANDLE,
            original_generation: GENERATION.into(),
        },
        ServiceCall::PromoteCandidate {
            handle: HANDLE,
            branch: "branch".into(),
            base: "base".into(),
            target: "target".into(),
        },
        ServiceCall::AbandonCandidate {
            handle: HANDLE,
            branch: "branch".into(),
            base: "base".into(),
            target: "target".into(),
        },
        ServiceCall::CandidateTransitionOutcome {
            original_id: HANDLE,
            original_generation: GENERATION.into(),
            transition: CandidateTransitionKind::Promote,
            branch: "branch".into(),
            base: "base".into(),
            target: "target".into(),
        },
        ServiceCall::SelectedAbandonOutcome {
            original_id: HANDLE,
            original_generation: GENERATION.into(),
            branch: "branch".into(),
            base: "base".into(),
            target: "target".into(),
        },
        ServiceCall::CandidateInventory {
            after: Some("branch".into()),
            limit: 1,
        },
        ServiceCall::CandidateRefStatus {
            branch: "branch".into(),
        },
        ServiceCall::AbandonCandidateRef {
            branch: "branch".into(),
            base: "base".into(),
            target: "target".into(),
        },
        ServiceCall::Ledger {
            operation: Box::new(LedgerOperation::Session {
                session_id: "session".into(),
            }),
        },
        ServiceCall::LedgerOutcome {
            original_id: HANDLE,
            original_generation: GENERATION.into(),
            proof: UsageProof::new_session("session"),
        },
        ServiceCall::BeginExport,
        ServiceCall::ExportPage {
            handle: HANDLE,
            cursor: Some(export_cursor),
        },
    ])
}

fn tag(value: &impl Serialize, key: &str) -> Result<String> {
    let encoded = serde_json::to_value(value)?;
    encoded
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("sample has no `{key}` tag: {encoded}"))
}

/// Every leaf operation as a routable call, labelled by its wire tags:
/// `<kind>` for direct calls, `view.<operation>` and `ledger.<operation>`.
fn labelled_calls() -> Result<Vec<(String, ServiceCall)>> {
    let mut calls = Vec::new();
    for call in service_call_samples()? {
        let kind = tag(&call, "kind")?;
        if kind != "view" && kind != "ledger" {
            calls.push((kind, call));
        }
    }
    for operation in view_operation_samples()? {
        let label = format!("view.{}", tag(&operation, "operation")?);
        calls.push((
            label,
            ServiceCall::View {
                candidate: None,
                operation: Box::new(operation),
            },
        ));
    }
    for operation in ledger_operation_samples() {
        let label = format!("ledger.{}", tag(&operation, "operation")?);
        calls.push((
            label,
            ServiceCall::Ledger {
                operation: Box::new(operation),
            },
        ));
    }
    Ok(calls)
}

/// Today's classification: (label, may_mutate, unit receipt method).
const CLASSIFICATION: &[(&str, bool, Option<&str>)] = &[
    ("retire_if_idle", true, None),
    ("try_acquire_dream_lease", false, None),
    ("append_message", true, Some("append_message")),
    ("history_window", false, None),
    ("notes", false, None),
    ("put_many", true, Some("put_many")),
    (
        "put_reasoning_summaries",
        true,
        Some("put_reasoning_summaries"),
    ),
    ("get", false, None),
    ("reconcile", false, None),
    ("revision", false, None),
    ("outcome", false, None),
    ("begin_candidate", true, None),
    ("candidate_outcome", false, None),
    ("promote_candidate", true, None),
    ("abandon_candidate", true, None),
    ("candidate_transition_outcome", false, None),
    ("selected_abandon_outcome", false, None),
    ("candidate_inventory", false, None),
    ("candidate_ref_status", false, None),
    ("abandon_candidate_ref", true, None),
    ("ledger_outcome", false, None),
    ("begin_export", false, None),
    ("export_page", false, None),
    ("view.append", true, Some("view.append")),
    ("view.append_message", true, Some("view.append_message")),
    (
        "view.append_session_message",
        true,
        Some("view.append_session_message"),
    ),
    ("view.checkpoint", true, Some("view.checkpoint")),
    (
        "view.checkpoint_session",
        true,
        Some("view.checkpoint_session"),
    ),
    ("view.history", false, None),
    ("view.history_window", false, None),
    ("view.session_history_window", false, None),
    ("view.session_history_window_after", false, None),
    ("view.session_catalog_page", false, None),
    ("view.session_catalog_record", false, None),
    ("view.create_session", true, Some("view.create_session")),
    ("view.rename_session", true, Some("view.rename_session")),
    ("view.remove_session", true, Some("view.remove_session")),
    ("view.restore_session", true, Some("view.restore_session")),
    ("view.fork_session", true, Some("view.fork_session")),
    ("view.public_transcript_page", false, None),
    ("view.session_source_snapshot", false, None),
    (
        "view.checkpoint_context_summary",
        true,
        Some("view.checkpoint_context_summary"),
    ),
    ("view.context_summary_cursor", false, None),
    ("view.context_summary_window", false, None),
    ("view.notes", false, None),
    ("view.forget_note", true, Some("view.forget_note")),
    ("view.put_many", true, Some("view.put_many")),
    ("view.get", false, None),
    ("view.clear", true, Some("view.clear")),
    ("view.reconcile", false, None),
    ("view.revision", false, None),
    ("view.revisions", false, None),
    ("view.status", false, None),
    ("ledger.mark_new_session", true, None),
    ("ledger.admit", true, None),
    ("ledger.observe", true, None),
    ("ledger.settle", true, None),
    ("ledger.session", false, None),
];

#[test]
fn every_operation_keeps_its_pinned_classification() -> Result<()> {
    let calls = labelled_calls()?;
    let sampled: Vec<&str> = calls.iter().map(|(label, _)| label.as_str()).collect();
    let pinned: Vec<&str> = CLASSIFICATION.iter().map(|(label, ..)| *label).collect();
    assert_eq!(sampled, pinned, "classification table and samples diverged");
    for ((label, call), (_, mutates, receipt)) in calls.iter().zip(CLASSIFICATION) {
        assert_eq!(
            call.may_mutate(),
            *mutates,
            "{label} mutation class changed"
        );
        assert_eq!(
            call.unit_receipt_method(),
            *receipt,
            "{label} unit receipt method changed"
        );
    }
    Ok(())
}
