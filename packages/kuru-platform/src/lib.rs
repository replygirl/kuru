//! Checked native mechanics shared by Kuru's domain packages.
//!
//! Filesystem operations retain object identity and explicit publication outcomes.
//! Windows-only process and IPC operations use owned handles behind safe APIs.
//! Database, shell, updater and application policy remain with their consumers.

pub mod fs;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;
