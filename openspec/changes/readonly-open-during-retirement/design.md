# Design

## Context

Measured (diagnosis `tmp/roadmap/fix-readonly-open-diagnosis-2026-09-30.md`):

- The failing reader is the direct facade `MemoryStore::open` (read-only), which goes to `store::MemoryStore::open` and reads only `<store>/endpoint.json`. If a TCP connect, authentication and identity check succeed it returns a borrowed `Server` with no owner. It reads no service record and no service lock.
- The store startup lock `locks/<hash>` is released when an open completes, so a serving or retiring owner does not hold it.
- With a forced pause in the retiring owner (scratch worktree, local macOS arm64), a reader pause of 1800 ms reproduced the CI message at `preferences.rs:73:55` twice and a pool timeout once; at borrow time the service owner lock and lifecycle lease were held, the service record absent and `endpoint.json` present.
- The test passed 20 of 20 locally without forcing. The CI timing window is inferred, not observed; the CI log has no owner or supervisor lines.

Inferred: the retiring generation's supervisor closes the borrowed connection in `stop_child` after `finish_owner`. #142 removed the 30 s idle window that had masked the unguaranteed borrow.

Signature 2, measured: the message comes only from `request_idle_retirement`'s `connect_local` context (service.rs, the single site carrying that string); run 36791870007 job 110146577721 gave a two-line cause chain; `close_paused` order is serve-loop break, `drop(listener)`, `record.retire`, `store.close`, then lock release, so between the loop break and the drop a connect queues in the backlog. Local macOS probe, four identical runs: the queued connect completes Ok, the handshake then fails `Broken pipe (os error 32)` (already mapped), and `request_idle_retirement` after the drop returns `Ok(None)`. At the instant after the listener drop the probe read record present, owner lock held.

Signature 2, inferred (no Linux run): on Linux the listener close releases queued embryo connections and sets ECONNRESET on the peer; tokio's connect then returns it via `take_error` after write readiness. `is_transport_unavailable` accepts only NotFound, ConnectionRefused and the Windows busy-instance marker, so it is fatal. Windows behavior is unknown.

## Goals / Non-Goals

**Goals:**
- Fixtures that inspect after a CLI command or TUI exit are ordered after the prior generation's Dolt reap and owner-lock release, using lock authority.
- The facade contract describes what the direct open does.
- A deterministic test pins the managed path's ordering.

- A reset or EOF at connect or handshake with a retiring owner is "no owner"; the lock then orders the caller after the reap, and a live faulty owner is still reported at the existing deadline.

**Non-Goals:**
- Changing `store.rs`/`server.rs` borrow semantics or teaching them the service locks.
- Retries, sleeps, raised deadlines.
- Closing the residual starter-election gap (follow-on).
- Windows verification.

## Decisions

- Use `open_managed_observed` (read-only) in the fixture, optionally after `memory::await_owner_exit`. Rejected: treating a present endpoint as absent while retiring (no local signal distinguishes retiring from starting; TOCTOU remains); a one-time record re-read on connection close (a retry hiding the fault, and the failure is after handshake).
- Keep the direct open's borrow semantics, since `terminal.rs` fault injection and in-TUI polling deliberately borrow a live attached owner.
- The deterministic test drives a real `ServiceOwner` with close pauses at `AfterEndpointRetire` and `AfterReap`, asserts the CI state at the pause (record absent, owner lock and lease held, `endpoint.json` present), starts the managed read-only open concurrently, releases the pauses event-driven, then asserts a read succeeds after the generation is reaped. A borrowing reader would read after the reap and fail.

- Add an arm `Err(error) if is_peer_closed(&error)` at both `connect_local` match sites: `request_idle_retirement` gives `Ok(RetirementReply::PeerClosed)` (an enum replacing `Option<bool>` so the trace can tell it from a missing endpoint), `try_attach_observed` gives `Ok(Err(AttachMiss::PeerClosed))`. Both go through one `connect_miss` classifier that reuses `is_peer_closed`; do not add ConnectionReset to `is_transport_unavailable`, which means nothing is listening, whereas a reset means a listener queued the connection and then closed it.
- Distinguishing a retiring owner from a live faulty one: the reset alone cannot (record present and owner lock held look the same), so it never decides. The owner lock released within the startup budget decides: a retiring owner reaps and releases it; a live faulty owner keeps it and the existing deadlines report it. `MaintenanceTrace` gains a peer-closed count separate from `unanswered`, and both the owner-still-active error and the owner-response deadline error (the deadline can fall inside a request) include the trace, so a live owner that keeps resetting is named as such. `AttachMiss::PeerClosed` already gives the client path this distinction in its readiness diagnostics. No retry is added and no deadline raised.
- Rejected: retiring the service record before dropping the listener so that "record present after a reset" means a fault (reopens a #142 contract pinned by `a_client_meeting_a_record_without_a_listener_elects_a_successor`, and still needs the lock to cover a crashed owner); widening `is_transport_unavailable`.
- Tests with no sleeps, `Notify` barriers and lock acquisition under `fixture_deadline`: (a) `maintenance_connect_queued_before_the_listener_dropped_waits_for_the_owner_lock`: owner paused at `BeforeListenerDrop`, `AfterListenerDrop`, `AfterReap`; pin `request_idle_retirement` and, under `cfg(unix)`, assert it is Pending so the window is established; release; assert record present and owner lock held at `AfterListenerDrop`; the request returns `Ok(None)`; `acquire_maintenance_permit` is Pending while `AfterReap` is held, then acquires. (b) `a_client_connect_queued_before_the_listener_dropped_is_a_peer_closed_miss`: same shape with `try_attach_observed` requiring `AttachMiss::PeerClosed`, optionally extended with `attach_or_start` electing a successor after the lock is released. (c) a platform-independent unit test: a `ConnectionReset` wrapped in the maintenance connect context is `is_peer_closed` and not `is_transport_unavailable`. (d) a unit test that the maintenance trace counts peer-closed apart from unanswered and the deadline error names it.

## Risks / Trade-offs

- The fixture change is not timing-independently testable in a child process → the `kuru-memory` test pins the relied-on contract; the fixture's own proof is repeated runs and CI.
- The paused interval must stay within the reader's startup budget → event-driven release, no timers.
- On macOS tests (a) and (b) take the connect-Ok then handshake-EPIPE path and pass before the fix, so only (c) pins the connect arm there; the ubuntu CI leg of (a) and (b) is the verification of the arm and is expected, by inference, to fail without it. Windows is unverified.
- Counting peer-closed outcomes changes diagnostics only; it must not change which outcomes are fatal.
- The residual starter-election gap remains → recorded as a follow-on, not hidden.
