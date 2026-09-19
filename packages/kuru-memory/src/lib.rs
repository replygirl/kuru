//! Private, versioned memory with an owned full-Dolt runtime.
mod catalog;
mod engine;
mod files;
mod migration;
mod progress;
pub mod provision;
pub mod server;
#[cfg(test)]
mod spawn_gate;
mod store;

pub use progress::{MemoryOpenProgress, MemoryOpenStage};
pub use store::purge::PurgeOutcome;
pub use store::{
    ActiveExportSnapshot, Candidate, ExportCursor, ExportPage, ExportProvenance, HistoryWindow,
    MemoryStatus, MemoryStore, MemoryView, OpenOptions, Revision, StorageRecord, StoredNote,
    UsageLedger,
};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
