//! A pool of persistent peers; scheduling policy is deterministic, never an LLM supervisor.
mod actor;
mod bus;
mod dream;
mod engine;
pub mod server;

pub use bus::PeerMessage;
pub use dream::{DreamProposal, DreamReport, undo_dream};
pub use engine::{
    CancellationToken, ControlledTurnOutput, Event, ForgetNoteResult, Harness, INTERRUPTION_ROLE,
    INTERRUPTION_TEXT, NotesView, ResponseOutcome, Session, StateReport, Topology, TurnLimitReason,
    TurnOutput, forget_note, project_scope, read_notes, turn_was_cancelled,
};

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

#[cfg(all(test, windows))]
mod windows_tool_tests;
