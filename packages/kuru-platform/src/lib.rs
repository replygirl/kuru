//! Checked native mechanics shared by Kuru's domain packages.
//!
//! Filesystem operations retain object identity and explicit publication outcomes.
//! Windows-only process and IPC operations use owned handles behind safe APIs.
//! Unix built-in shells may use a narrow fresh-process-group owner which keeps
//! standard-child identity through ordered termination, reaping, and absence
//! observation. It is not a general process supervisor or a sandbox.
//! Database, shell, updater and application policy remain with their consumers.

pub mod fs;

#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;
