//! A pool of persistent peers; scheduling policy is deterministic, never an LLM supervisor.
mod actor;
mod bus;
mod dream;
mod engine;
pub mod server;

pub use bus::PeerMessage;
pub use dream::{DreamProposal, DreamReport};
pub use engine::{Event, Harness, Session, StateReport, Topology, TurnOutput, project_scope};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod review_tests;

#[cfg(test)]
mod preferences_tests;

#[cfg(test)]
mod dolt_tests;
