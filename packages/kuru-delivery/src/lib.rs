//! Native archive installation and package-owned delivery tooling.

pub mod archive;
#[cfg(feature = "tooling")]
pub mod bundle;
pub mod command;
#[cfg(feature = "tooling")]
pub mod docs;
#[cfg(feature = "tooling")]
pub mod notes;
#[cfg(feature = "tooling")]
pub mod release;
#[cfg(feature = "tooling")]
pub mod repo;
mod staging;
pub mod targets;
#[cfg(windows)]
pub mod update;
