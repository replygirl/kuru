# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Coordination, not a block

Neither item below is a change slug that exists on this branch, so cospec cannot ledger it (a dangling slug fails `cospec validate --strict`: `blockers/dangling-ref`, observed when `memory-derived-waits` was tried under `Soft-blocked by`). Both are tracked here and in `tasks.md` task 1.2 instead.

- PR 1, branch `test/memory-derived-waits` (change `memory-derived-waits`, kuru-memory test waits): its exposure commit makes `OPERATION_TIMEOUT` public as `kuru_memory::service::OPERATION_TIMEOUT` under `test-support` (its task 5.1: `#[cfg(any(test, feature = "test-support"))] pub use rpc::OPERATION_TIMEOUT;` beside the `pub use rpc::{..}` at `service.rs:28`, no value change). Every `OPERATION_TIMEOUT` derivation in this change needs it. Until PR 1 merges, cherry-pick exactly that one commit onto this branch and record its sha on the next line; after PR 1 merges, the orchestrator rebases onto main and the duplicate drops out. As of 2026-10-03 23:06 (-05:00) the branch is at 63942438 and holds only its `chore(cospec): scope memory-derived-waits` commit, so the exposure commit does not exist yet and there is no sha to record: implement everything that does not need `OPERATION_TIMEOUT` first and leave those derivations for last.
  - Exposure commit sha: not yet available.
- PR #208 (`fix/instrumented-child-outlives-test`, open, another assistant) edits `apps/kuru-tui/tests/support/terminal.rs` lines 174-210 (the spawn ledger in `Terminal::spawn`, +15 lines). This change edits that file only at the constants and `startup_timeout` (lines 19-25 and 107-115) and at `wait_exit` (line 588), so the hunks do not overlap; whichever merges second rebases. If #208 merges first, every line below 210 in that file shifts by about 15 (inference, not measured): re-check `wait_text` (:352), `submit` (:440) and the `wait_exit` drain (:588) before editing. #208 also touches `kuru-memory` product files (`service.rs`, `server.rs`, `test_support.rs`) that this change does not edit.
