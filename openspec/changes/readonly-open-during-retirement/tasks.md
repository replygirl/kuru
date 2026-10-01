# Tasks

## 1. Regression test

- [ ] 1.1 Add `managed_inspection_meeting_a_retiring_owner_waits_for_its_reap_and_reads_its_own_generation` to the `kuru-memory` service tests, event-driven with no sleeps, and verify it passes with the fix path and that a direct (borrowing) open in the same state fails the post-reap read

## 2. Fixtures

- [ ] 2.1 Convert `Sandbox::preferences()` in `apps/kuru-tui/tests/preferences.rs` to `open_managed_observed` read-only after `await_owner_exit`, and verify both tests in the file pass
- [ ] 2.2 Audit `terminal.rs`, `embedded_runtime.rs` and `mise_acceptance.rs` read-only direct opens, convert those that run after a CLI or TUI exit, keep the deliberate live-owner borrows, and verify the affected tests pass

## 3. Contract

- [ ] 3.1 Correct the `MemoryStore::open` doc comment in `facade.rs` and verify docs and lint pass

## 4. Gates

- [ ] 4.1 Run format, lint (including the Windows target), the affected package tests, and `mise run cospec -- validate readonly-open-during-retirement --strict`, then record evidence in verification.md
