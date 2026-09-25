//! A pool of persistent peers; scheduling policy is deterministic, never an LLM supervisor.
mod actor;
mod bus;
mod context_compaction;
mod dream;
mod engine;
mod event;
mod progress;
pub mod server;

pub use bus::PeerMessage;
pub use dream::{DreamProposal, DreamReport, undo_dream};
pub use engine::{
    CancellationToken, ControlledTurnOutput, ForgetNoteResult, Harness, INTERRUPTION_ROLE,
    INTERRUPTION_TEXT, NotesView, ResponseOutcome, Session, Topology, TurnOutput, forget_note,
    project_scope, read_notes, turn_was_cancelled,
};
pub use event::{Event, StateReport, ToolObservation, ToolOutcome, TurnLimitReason};
pub use progress::{ContextSnapshot, FacingProgress, RequestContext};

#[cfg(test)]
fn test_receipt(message: &kuru_core::Message) -> anyhow::Result<serde_json::Value> {
    use kuru_core::ContentBlock;
    use serde_json::json;
    match message.blocks.as_slice() {
        [
            ContentBlock::ToolResult {
                call_id,
                output,
                is_error,
            },
        ] => Ok(json!({
            "call_id": call_id,
            "output": output,
            "is_error": is_error,
        })),
        [ContentBlock::Text { text }] => Ok(serde_json::from_str(text)?),
        _ => anyhow::bail!("expected one typed or legacy tool receipt"),
    }
}

#[cfg(test)]
fn test_receipt_output(message: &kuru_core::Message) -> Option<String> {
    test_receipt(message)
        .ok()?
        .get("output")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod review_tests;

#[cfg(test)]
mod preferences_tests;

#[cfg(test)]
mod dolt_tests;

#[cfg(test)]
mod notes_tests;

#[cfg(test)]
mod mode_baseline_tests;

#[cfg(test)]
mod mode_policy_tests;

#[cfg(test)]
mod mode_visibility_tests;

#[cfg(test)]
mod mode_memory_tests;

#[cfg(test)]
mod mode_consolidation_tests;

#[cfg(test)]
mod progress_tests;

#[cfg(test)]
mod permission_tests;

#[cfg(test)]
mod accounting_tests;

#[cfg(test)]
mod reasoning_summary_tests;

#[cfg(all(test, windows))]
mod windows_tool_tests;
