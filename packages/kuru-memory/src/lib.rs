//! Private, versioned memory with an owned full-Dolt runtime.
mod catalog;
mod engine;
mod files;
mod migration;
pub mod provision;
pub mod server;
mod store;

pub use store::{
    ActiveExportSnapshot, Candidate, ExportCursor, ExportPage, ExportProvenance, MemoryStatus,
    MemoryStore, MemoryView, OpenOptions, Revision, StorageRecord, StoredNote,
};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
