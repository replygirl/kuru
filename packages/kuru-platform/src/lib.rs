//! Checked native mechanics shared by Kuru's domain packages.
//!
//! Filesystem operations retain object identity and explicit publication outcomes.
//! Windows-only process and IPC operations use owned handles behind safe APIs.
//! Unix built-in shells may use a narrow fresh-process-group owner which keeps
//! standard-child identity through ordered termination, reaping, and absence
//! observation. It is not a general process supervisor or a sandbox.
//! Unix failure paths may describe a process tree through a bounded, read-only
//! `ps` snapshot whose numeric IDs are diagnostic text and never acted on.
//! Database, shell, updater and application policy remain with their consumers.

pub mod fs;
pub mod secret;

#[cfg(unix)]
pub mod local_ipc;

#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;
