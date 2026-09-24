//! Private, versioned memory with an owned full-Dolt runtime.
mod catalog;
mod engine;
mod facade;
mod files;
mod migration;
mod progress;
pub mod provision;
pub mod server;
pub mod service;
#[cfg(test)]
mod spawn_gate;
mod store;

pub use facade::{
    ActiveExportSnapshot, Candidate, CandidateTransitionRecovery, CandidateTransitionResolution,
    CandidateUnitRecovery, DreamLease, ExportCursor, ExportPage, MemoryStore, MemoryView,
    SelectedAbandonResolution, SelectedAbandonUncertain, UsageLedger,
};
pub use progress::{MemoryOpenProgress, MemoryOpenStage};
pub use store::purge::PurgeOutcome;
pub use store::{
    CandidateConflict, CandidateInventoryPage, CandidateRefRefusal, CandidateRefRejected,
    CandidateRefState, CandidateRefStatus, ContextSummaryCursor, ContextSummaryItem,
    ContextSummaryRecord, ContextSummaryStale, ContextSummaryWindow, ExportProvenance,
    HistoryWindow, MemoryStatus, OpenOptions, ReasoningSummaryConflict, ReasoningSummaryRecord,
    Revision, SequencedMessage, SessionSourceSnapshot, StorageRecord, StoredNote, UsageProof,
};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
