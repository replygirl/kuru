# Tasks

## 1. Regression test

- [x] 1.1 Add `managed_inspection_meeting_a_retiring_owner_waits_for_its_reap_and_reads_its_own_generation` to the `kuru-memory` service tests, event-driven with no sleeps, and verify it passes with the fix path and that a direct (borrowing) open in the same state fails the post-reap read

- [x] 1.2 Add the signature 2 tests: `maintenance_connect_queued_before_the_listener_dropped_waits_for_the_owner_lock`, `a_client_connect_queued_before_the_listener_dropped_is_a_peer_closed_miss`, and the platform-independent routing unit test; record which fail without the fix on each host

## 2. Fixtures

- [x] 2.1 Convert `Sandbox::preferences()` in `apps/kuru-tui/tests/preferences.rs` to `open_managed_observed` read-only after `await_owner_exit`, and verify both tests in the file pass
- [x] 2.2 Audit `terminal.rs`, `embedded_runtime.rs` and `mise_acceptance.rs` read-only direct opens, convert those that run after a CLI or TUI exit, keep the deliberate live-owner borrows, and verify the affected tests pass

## 3. Contract

- [x] 3.1 Correct the `MemoryStore::open` doc comment in `facade.rs` and verify docs and lint pass

## 3b. Connect-reset mapping

- [x] 3.2 Map a peer-closed connect error in `request_idle_retirement` (to `RetirementReply::PeerClosed`) and `try_attach_observed` (to `AttachMiss::PeerClosed`); count peer-closed apart from unanswered in `MaintenanceTrace` and name the trace in both maintenance deadline errors (owner still active, owner-response deadline); leave the close order, deadlines and `is_transport_unavailable` unchanged, and verify tests 1.2

## 4. Gates

- [x] 4.1 Run format, lint (including the Windows target), the affected package tests, and `mise run cospec -- validate readonly-open-during-retirement --strict`, then record evidence in verification.md
