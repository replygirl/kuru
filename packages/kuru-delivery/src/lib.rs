//! Native archive installation and package-owned delivery tooling.

#[cfg(feature = "tooling")]
pub mod advisory;
pub mod archive;
#[cfg(feature = "tooling")]
pub mod bundle;
pub mod command;
#[cfg(feature = "tooling")]
pub mod coverage;
#[cfg(feature = "tooling")]
pub mod docs;
// Shared with the advisory integration tests and scanner fixture.
#[cfg(all(test, feature = "tooling"))]
#[path = "../tests/support/fixture_git.rs"]
mod fixture_git;
// The fixtures' launch budgets, which name this crate as an integration
// test does.
#[cfg(all(test, feature = "tooling"))]
extern crate self as kuru_delivery;
#[cfg(all(test, feature = "tooling"))]
#[path = "../tests/support/launch_budget.rs"]
mod launch_budget;
#[cfg(any(feature = "tooling", windows))]
mod lease;
#[cfg(feature = "tooling")]
pub mod mise_isolation;
#[cfg(feature = "tooling")]
pub mod notes;
#[cfg(feature = "tooling")]
pub mod open_time;
#[cfg(feature = "tooling")]
pub mod published;
#[cfg(feature = "tooling")]
pub mod published_windows;
#[cfg(feature = "tooling")]
pub mod release;
#[cfg(feature = "tooling")]
pub mod repo;
pub mod shell_support;
mod staging;
pub mod targets;
#[cfg(windows)]
pub mod update;
// The Windows handoff's waits; public only to the maintainer fixtures that
// bound a previous updater's launch by them.
#[cfg(feature = "tooling")]
pub mod update_budget;
#[cfg(all(windows, not(feature = "tooling")))]
mod update_budget;
