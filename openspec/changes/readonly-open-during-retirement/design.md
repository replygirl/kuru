# Design

## Context

Measured (diagnosis `tmp/roadmap/fix-readonly-open-diagnosis-2026-09-30.md`):

- The failing reader is the direct facade `MemoryStore::open` (read-only), which goes to `store::MemoryStore::open` and reads only `<store>/endpoint.json`. If a TCP connect, authentication and identity check succeed it returns a borrowed `Server` with no owner. It reads no service record and no service lock.
- The store startup lock `locks/<hash>` is released when an open completes, so a serving or retiring owner does not hold it.
- With a forced pause in the retiring owner (scratch worktree, local macOS arm64), a reader pause of 1800 ms reproduced the CI message at `preferences.rs:73:55` twice and a pool timeout once; at borrow time the service owner lock and lifecycle lease were held, the service record absent and `endpoint.json` present.
- The test passed 20 of 20 locally without forcing. The CI timing window is inferred, not observed; the CI log has no owner or supervisor lines.

Inferred: the retiring generation's supervisor closes the borrowed connection in `stop_child` after `finish_owner`. #142 removed the 30 s idle window that had masked the unguaranteed borrow.

## Goals / Non-Goals

**Goals:**
- Fixtures that inspect after a CLI command or TUI exit are ordered after the prior generation's Dolt reap and owner-lock release, using lock authority.
- The facade contract describes what the direct open does.
- A deterministic test pins the managed path's ordering.

**Non-Goals:**
- Changing `store.rs`/`server.rs` borrow semantics or teaching them the service locks.
- Retries, sleeps, raised deadlines.
- Closing the residual starter-election gap (follow-on).
- Windows verification.

## Decisions

- Use `open_managed_observed` (read-only) in the fixture, optionally after `memory::await_owner_exit`. Rejected: treating a present endpoint as absent while retiring (no local signal distinguishes retiring from starting; TOCTOU remains); a one-time record re-read on connection close (a retry hiding the fault, and the failure is after handshake).
- Keep the direct open's borrow semantics, since `terminal.rs` fault injection and in-TUI polling deliberately borrow a live attached owner.
- The deterministic test drives a real `ServiceOwner` with close pauses at `AfterEndpointRetire` and `AfterReap`, asserts the CI state at the pause (record absent, owner lock and lease held, `endpoint.json` present), starts the managed read-only open concurrently, releases the pauses event-driven, then asserts a read succeeds after the generation is reaped. A borrowing reader would read after the reap and fail.

## Risks / Trade-offs

- The fixture change is not timing-independently testable in a child process → the `kuru-memory` test pins the relied-on contract; the fixture's own proof is repeated runs and CI.
- The paused interval must stay within the reader's startup budget → event-driven release, no timers.
- The residual starter-election gap remains → recorded as a follow-on, not hidden.
