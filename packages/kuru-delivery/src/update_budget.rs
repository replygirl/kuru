//! The Windows update handoff's waits, kept outside the Windows-only updater so
//! maintainer fixtures on every host can bound an updater launch by them. The
//! fixtures compose the parent's series from these and its wait sites.

use std::time::Duration;

/// Connection establishment, each control frame already in flight and each
/// pipe close.
pub const STARTUP: Duration = Duration::from_secs(10);
/// The authenticated helper verifies and durably copies full embedded images
/// before acknowledging publication. That work has its own finite allowance;
/// connection establishment and a frame already in flight keep their short limit.
pub const PUBLICATION: Duration = Duration::from_secs(120);
/// Draining the helper's diagnostics and waiting for its exit after a failure.
pub const CLEANUP: Duration = Duration::from_secs(10);
