//! Pins the per-operation contract and the memory service wire surface.

use super::*;
use crate::service::{PROTOCOL_MAJOR, PROTOCOL_MINOR};
use kuru_core::{Usage, UsagePhase};
use serde_json::json;

const HANDLE: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
const GENERATION: &str = "01234567-89ab-cdef-0123-456789abcdef";

fn message() -> Message {
    Message::text("user", "hello")
}

fn values() -> Vec<(String, Value)> {
    vec![("key".into(), json!({"value": true}))]
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

/// Mutating calls without a unit receipt, and the typed proof each keeps.
const TYPED_PROOFS: &[(&str, Receipt)] = &[
    ("retire_if_idle", Receipt::None),
    ("begin_candidate", Receipt::CandidateCreation),
    ("promote_candidate", Receipt::CandidateTransition),
    ("abandon_candidate", Receipt::CandidateTransition),
    ("abandon_candidate_ref", Receipt::SelectedAbandon),
    ("ledger.mark_new_session", Receipt::UsageProof),
    ("ledger.admit", Receipt::UsageProof),
    ("ledger.observe", Receipt::UsageProof),
    ("ledger.settle", Receipt::UsageProof),
];

#[test]
fn every_operation_contract_decides_receipt_and_reply_budget() -> Result<()> {
    let calls = labelled_calls()?;
    for ((label, call), &(_, mutates, unit)) in calls.iter().zip(CLASSIFICATION) {
        let typed = TYPED_PROOFS
            .iter()
            .find(|(typed, _)| typed == label)
            .map(|(_, receipt)| *receipt);
        let expected = match (mutates, unit, typed) {
            (true, Some(method), None) => Receipt::Unit(method),
            (true, None, Some(receipt)) => receipt,
            (false, None, None) => Receipt::None,
            _ => bail!("{label} has an inconsistent pinned classification"),
        };
        let contract = call.contract();
        assert_eq!(contract.receipt, expected, "{label} receipt class changed");
        assert_eq!(
            contract.reply,
            ReplyBudget::Operation,
            "{label} reply budget changed"
        );
    }
    assert_eq!(ReplyBudget::Operation.deadline(), OPERATION_TIMEOUT);
    for (typed, _) in TYPED_PROOFS {
        ensure!(
            calls.iter().any(|(label, _)| label == typed),
            "typed proof row {typed} names no sampled operation"
        );
    }
    Ok(())
}

#[test]
fn only_idle_retirement_mutates_without_a_durable_receipt() -> Result<()> {
    for (label, call) in labelled_calls()? {
        let contract = call.contract();
        match (contract.mutation, contract.receipt) {
            (Mutation::Write, Receipt::None) => assert_eq!(
                label, "retire_if_idle",
                "{label} mutates without a durable receipt; a lost reply could not be proven"
            ),
            (Mutation::Read, Receipt::None) => {}
            (Mutation::Write, _) => {}
            (Mutation::Read, receipt) => {
                panic!("{label} is read-only but registers receipt {receipt:?}")
            }
        }
    }
    Ok(())
}

#[test]
fn attachment_resources_include_the_dream_lease_but_handles_do_not() -> Result<()> {
    let mut state = AttachmentState::default();
    assert!(!state.holds_handles());
    assert!(!state.holds_resources());
    state.dream_lease = Some(
        Arc::new(tokio::sync::Mutex::new(()))
            .try_lock_owned()
            .context("fresh dream lease mutex")?,
    );
    assert!(!state.holds_handles());
    assert!(state.holds_resources());
    Ok(())
}

// ---------------------------------------------------------------------------
// Protocol pin
// ---------------------------------------------------------------------------

const PROTOCOL_SURFACE: &str = include_str!("protocol-surface.txt");
const PROTOCOL_SURFACE_PATH: &str = "packages/kuru-memory/src/service/rpc/protocol-surface.txt";
const BLESS_PROTOCOL_PIN: &str = "KURU_BLESS_PROTOCOL_PIN";
const UNKNOWN_VARIANT: &str = "__kuru_protocol_pin_unknown_variant__";

/// The derive's own variant list, read from serde's unknown-variant refusal,
/// so a variant without a sample cannot go unnoticed.
fn serde_variants<T: serde::de::DeserializeOwned>(name: &str, probe: Value) -> Result<Vec<String>> {
    let Err(error) = serde_json::from_value::<T>(probe) else {
        bail!("{name} accepted the unknown-variant probe");
    };
    let error = error.to_string();
    let listed = error
        .strip_prefix(&format!("unknown variant `{UNKNOWN_VARIANT}`, expected "))
        .with_context(|| format!("{name} did not list its variants: {error}"))?;
    let mut variants: Vec<String> = listed
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    ensure!(!variants.is_empty(), "{name} listed no variants: {error}");
    variants.sort();
    Ok(variants)
}

fn unknown_tag(tag: &str) -> Value {
    json!({ tag: UNKNOWN_VARIANT })
}

fn unknown_adjacent(tag: &str) -> Value {
    json!({ tag: UNKNOWN_VARIANT, "value": null })
}

fn unknown_unit() -> Value {
    Value::String(UNKNOWN_VARIANT.into())
}

fn wire_enums() -> Result<Vec<(&'static str, Vec<String>)>> {
    Ok(vec![
        (
            "service_call",
            serde_variants::<ServiceCall>("ServiceCall", unknown_tag("kind"))?,
        ),
        (
            "view_operation",
            serde_variants::<ViewOperation>("ViewOperation", unknown_tag("operation"))?,
        ),
        (
            "ledger_operation",
            serde_variants::<LedgerOperation>("LedgerOperation", unknown_tag("operation"))?,
        ),
        (
            "candidate_transition_kind",
            serde_variants::<CandidateTransitionKind>("CandidateTransitionKind", unknown_unit())?,
        ),
        (
            "content_block",
            serde_variants::<kuru_core::ContentBlock>("ContentBlock", unknown_tag("type"))?,
        ),
        ("mode", serde_variants::<Mode>("Mode", unknown_unit())?),
        (
            "session_lifecycle_state",
            serde_variants::<crate::SessionLifecycleState>(
                "SessionLifecycleState",
                unknown_unit(),
            )?,
        ),
        (
            "session_turn_checkpoint",
            serde_variants::<crate::SessionTurnCheckpoint>(
                "SessionTurnCheckpoint",
                unknown_tag("transition"),
            )?,
        ),
        (
            "public_turn_settlement",
            serde_variants::<crate::store::PublicTurnSettlement>(
                "PublicTurnSettlement",
                unknown_unit(),
            )?,
        ),
        (
            "public_transcript_position",
            serde_variants::<crate::store::PublicTranscriptPosition>(
                "PublicTranscriptPosition",
                unknown_tag("kind"),
            )?,
        ),
        (
            "usage_phase",
            serde_variants::<UsagePhase>("UsagePhase", unknown_unit())?,
        ),
        (
            "invocation_outcome",
            serde_variants::<InvocationOutcome>("InvocationOutcome", unknown_unit())?,
        ),
        (
            "usage_proof",
            serde_variants::<UsageProof>("UsageProof", unknown_tag("kind"))?,
        ),
        (
            "service_response",
            serde_variants::<ServiceResponse>("ServiceResponse", unknown_adjacent("status"))?,
        ),
        (
            "service_value",
            serde_variants::<ServiceValue>("ServiceValue", unknown_adjacent("kind"))?,
        ),
        (
            "service_fault",
            serde_variants::<ServiceFault>("ServiceFault", unknown_unit())?,
        ),
        (
            "outcome_status",
            serde_variants::<OutcomeStatus>("OutcomeStatus", unknown_unit())?,
        ),
        (
            "candidate_creation_outcome",
            serde_variants::<CandidateCreationOutcome>(
                "CandidateCreationOutcome",
                unknown_tag("status"),
            )?,
        ),
        (
            "candidate_transition_result",
            serde_variants::<CandidateTransitionResult>(
                "CandidateTransitionResult",
                unknown_adjacent("status"),
            )?,
        ),
        (
            "candidate_ref_refusal",
            serde_variants::<CandidateRefRefusal>("CandidateRefRefusal", unknown_unit())?,
        ),
        (
            "session_lifecycle_refusal",
            serde_variants::<crate::SessionLifecycleRefusal>(
                "SessionLifecycleRefusal",
                unknown_unit(),
            )?,
        ),
        (
            "session_turn_refusal",
            serde_variants::<crate::SessionTurnRefusal>("SessionTurnRefusal", unknown_unit())?,
        ),
    ])
}

/// JSON structure with leaf values replaced by their types and object keys
/// sorted. Arrays list each distinct element shape once.
fn shape(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(_) => "bool".into(),
        Value::Number(_) => "number".into(),
        Value::String(_) => "string".into(),
        Value::Array(items) => {
            let mut shapes: Vec<String> = Vec::new();
            for item in items {
                let item = shape(item);
                if !shapes.contains(&item) {
                    shapes.push(item);
                }
            }
            format!("[{}]", shapes.join("|"))
        }
        Value::Object(fields) => {
            let mut fields: Vec<String> = fields
                .iter()
                .map(|(key, value)| format!("{key}:{}", shape(value)))
                .collect();
            fields.sort();
            format!("{{{}}}", fields.join(","))
        }
    }
}

/// `<tag> <shape without the tag>` for one internally tagged sample.
fn tagged_shape(value: &impl Serialize, key: &str) -> Result<(String, String)> {
    let mut encoded = serde_json::to_value(value)?;
    let fields = encoded
        .as_object_mut()
        .context("tagged sample is not an object")?;
    let tag = fields
        .remove(key)
        .and_then(|tag| tag.as_str().map(str::to_owned))
        .with_context(|| format!("sample has no `{key}` tag"))?;
    Ok((tag, shape(&encoded)))
}

fn sampled(
    name: &str,
    key: &str,
    samples: &[impl Serialize],
    variants: &[String],
    lines: &mut Vec<String>,
) -> Result<()> {
    let mut tags = Vec::new();
    for sample in samples {
        let (tag, shape) = tagged_shape(sample, key)?;
        lines.push(format!("sample {name}.{tag} {shape}"));
        tags.push(tag);
    }
    tags.sort();
    ensure!(
        tags == variants,
        "{name} samples in packages/kuru-memory/src/service/rpc/contract_tests.rs must cover \
         exactly the wire variants.\n  sampled: {tags:?}\n  wire:    {variants:?}\nAdd one \
         sample per new variant with every optional field populated."
    );
    Ok(())
}

fn wire_surface() -> Result<String> {
    let enums = wire_enums()?;
    let variants = |name: &str| -> Result<&[String]> {
        enums
            .iter()
            .find(|(enumeration, _)| *enumeration == name)
            .map(|(_, variants)| variants.as_slice())
            .with_context(|| format!("missing wire enum {name}"))
    };
    let mut lines = vec![
        "# Memory service wire surface, generated by service::rpc::contract_tests.".to_owned(),
        "# Do not edit by hand; see docs/development.md, \"Memory service protocol\".".to_owned(),
        format!("protocol {}.{}", PROTOCOL_MAJOR, PROTOCOL_MINOR),
        String::new(),
    ];
    for (name, variants) in &enums {
        lines.push(format!("enum {name}: {}", variants.join(", ")));
    }
    lines.push(String::new());
    sampled(
        "service_call",
        "kind",
        &service_call_samples()?,
        variants("service_call")?,
        &mut lines,
    )?;
    sampled(
        "view_operation",
        "operation",
        &view_operation_samples()?,
        variants("view_operation")?,
        &mut lines,
    )?;
    sampled(
        "ledger_operation",
        "operation",
        &ledger_operation_samples(),
        variants("ledger_operation")?,
        &mut lines,
    )?;
    let request = ServiceRequest::with_id(GENERATION, HANDLE, ServiceCall::Revision);
    lines.push(format!(
        "envelope request {}",
        shape(&serde_json::to_value(&request)?)
    ));
    let reply = ServiceReply {
        id: HANDLE,
        generation: GENERATION.into(),
        response: ServiceResponse::Success(Box::new(ServiceValue::Unit)),
    };
    lines.push(format!(
        "envelope reply {}",
        shape(&serde_json::to_value(&reply)?)
    ));
    let rejected = ServiceResponse::Rejected(ServiceFault::StorageFailed);
    lines.push(format!(
        "envelope rejected {}",
        shape(&serde_json::to_value(&rejected)?)
    ));
    lines.push(String::new());
    Ok(lines.join("\n"))
}

fn recorded_protocol(surface: &str) -> Option<&str> {
    surface
        .lines()
        .find_map(|line| line.strip_prefix("protocol "))
}

#[test]
fn protocol_surface_matches_the_pinned_protocol_version() -> Result<()> {
    let current = wire_surface()?;
    if current == PROTOCOL_SURFACE {
        return Ok(());
    }
    let version = format!("{}.{}", PROTOCOL_MAJOR, PROTOCOL_MINOR);
    let recorded = recorded_protocol(PROTOCOL_SURFACE).unwrap_or("none");
    let bumped = recorded != version;
    if std::env::var_os(BLESS_PROTOCOL_PIN).is_some_and(|value| value == "1") {
        ensure!(
            bumped,
            "refusing to record a changed memory service wire surface under unchanged protocol \
             {version}. Bump PROTOCOL_MINOR in packages/kuru-memory/src/service.rs first."
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/service/rpc/protocol-surface.txt");
        std::fs::write(&path, &current).with_context(|| format!("write {}", path.display()))?;
        return Ok(());
    }
    let removed = PROTOCOL_SURFACE
        .lines()
        .filter(|line| !current.lines().any(|current| current == *line))
        .map(|line| format!("  - {line}"));
    let added = current
        .lines()
        .filter(|line| !PROTOCOL_SURFACE.lines().any(|recorded| recorded == *line))
        .map(|line| format!("  + {line}"));
    let changes: Vec<String> = removed.chain(added).collect();
    let action = if bumped {
        format!(
            "PROTOCOL_MINOR is already {version} (fixture records {recorded}); regenerate the \
             fixture"
        )
    } else {
        format!(
            "The wire changed under unchanged protocol {version}. Bump PROTOCOL_MINOR in \
             packages/kuru-memory/src/service.rs so older owners refuse the newer client at the \
             handshake instead of failing to decode its request, then regenerate the fixture"
        )
    };
    bail!(
        "memory service wire surface differs from {PROTOCOL_SURFACE_PATH}:\n{}\n\n{action} with\n  \
         {BLESS_PROTOCOL_PIN}=1 mise run //packages/kuru-memory:test -- protocol_surface\nand \
         review the fixture diff. Shared kuru-core types carried on the wire (messages, usage, \
         modes) trigger this check deliberately.",
        changes.join("\n")
    );
}
