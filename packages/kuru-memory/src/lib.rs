//! Private, versioned memory with an owned full-Dolt runtime.
mod catalog;
mod engine;
mod files;
mod migration;
pub mod provision;
pub mod server;
mod store;

pub use store::{Candidate, MemoryStatus, MemoryStore, MemoryView, OpenOptions, Revision};

/// Isolated real-engine fixtures shared by the Rust workspace's behavioral tests.
pub mod test_support {
    use crate::OpenOptions;
    pub use crate::files::PrivateTemp as TempDir;
    use anyhow::Result;
    use std::path::PathBuf;

    /// Private real-filesystem root on every supported OS, including a protected
    /// Windows DACL instead of tempfile's ordinary inherited permissions.
    pub fn tempdir() -> Result<TempDir> {
        TempDir::new("kuru-fixture-", None)
    }

    pub fn open_options(data_dir: PathBuf, project_scope: String) -> Result<OpenOptions> {
        let mut options = OpenOptions::new(data_dir, project_scope);
        options.config.cache_dir = Some(crate::store::test_cache());
        options.config.offline = true;
        options.supervisor = Some(crate::store::test_supervisor()?);
        Ok(options)
    }

    pub fn cache_dir() -> PathBuf {
        crate::store::test_cache()
    }
}
