# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

<!--
Not a cospec dependency (no other active change exists to reference), but
recorded here for the orchestrating session per this change's own finding:
the failure family cannot be eliminated by any change confined to
`packages/kuru-memory/src/store/creation_template/tests.rs` or
`open_tests.rs`. The chokepoint is `MemoryStore::open_temporary` (store.rs)
calling `Server::open_with_guard` → `open_inner_with_probe_delay`
(server.rs) with `_test_spawn_guard: None` on every one of the ~90
`temporary()`/`temporary_cold()` call sites across the crate. Closing it
needs a `spawn_gate::spawning()` guard around that open, which changes
suite-wide gate wait behavior (every `temporary()` open then queues behind
any held write guard) — a scope and design decision for the orchestrating
session, not made in this change. See proposal.md.
-->
