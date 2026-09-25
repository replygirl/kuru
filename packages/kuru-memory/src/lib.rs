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
    CandidateRefState, CandidateRefStatus, ContextSummaryCheckpoint, ContextSummaryCursor,
    ContextSummaryItem, ContextSummaryRecord, ContextSummaryStale, ContextSummaryWindow,
    ExportProvenance, FORK_PROVENANCE_RECORD_FORMAT, HistoryWindow, LEGACY_PREFIX_RECORD_FORMAT,
    LegacySessionTurnResume, LegacyTranscriptPrefix, MAX_SESSION_LABEL_BYTES, MemoryStatus,
    OpenOptions, PUBLIC_TURN_RECORD_FORMAT, PublicTranscriptCursor, PublicTranscriptEntry,
    PublicTranscriptPage, PublicTranscriptPosition, PublicTurnKind, PublicTurnRecord,
    PublicTurnSettlement, ReasoningSummaryConflict, ReasoningSummaryRecord, Revision,
    SESSION_CATALOG_RECORD_FORMAT, SESSION_LIFECYCLE_OUTCOME_FORMAT, SequencedMessage,
    SessionCatalogCursor, SessionCatalogPage, SessionCatalogRecord, SessionForkProvenance,
    SessionHistoryWindowAfter, SessionLifecycleOutcome, SessionLifecycleRefusal,
    SessionLifecycleRejected, SessionLifecycleState, SessionModeCheckpoint, SessionSourceSnapshot,
    SessionTurnCheckpoint, SessionTurnRefusal, SessionTurnRejected, StorageRecord, StoredNote,
    UsageProof, context_summary_id, public_turn_continuation_node_id,
    public_turn_legacy_continuation_node_id, public_turn_node_id,
};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
