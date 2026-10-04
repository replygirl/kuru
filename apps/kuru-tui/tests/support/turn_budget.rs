//! The runtime's whole-turn budget, restated once for every test that waits on
//! a turn: the integration tests include this file, and so does the in-crate
//! src/ui/runtime_tests.rs, which cannot reach the Unix-only terminal support.

use std::time::Duration;

/// What the runtime allows a turn after its cancellation, for accepted memory
/// work and the answer race to settle: 35 s (kuru-runtime src/server.rs:197;
/// docs/protocols.md, "allows up to 35 more seconds").
pub const TURN_SETTLEMENT: Duration = Duration::from_secs(35);

/// The runtime's whole-turn budget: a turn may run for 600 s (`run_controlled`
/// under `tokio::time::timeout(Duration::from_secs(600), ..)`, kuru-runtime
/// src/server.rs:190; docs/protocols.md, "up to 10 minutes"), then
/// `TURN_SETTLEMENT`. The runtime enforces it on A2A ingress; the TUI and
/// `kuru run` reach the same `run_controlled_inner` through
/// `run_local_controlled`, so it is the product's stated bound for one turn on
/// their paths too. Restated because the runtime keeps both values inline.
pub const TURN_BUDGET: Duration = Duration::from_secs(600).saturating_add(TURN_SETTLEMENT);
