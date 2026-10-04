# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Coordination, not a block

Neither item below is a change slug that exists on this branch, so cospec cannot ledger it (a dangling slug fails `cospec validate --strict`: `blockers/dangling-ref`, observed when `memory-derived-waits` was tried under `Soft-blocked by`). Both are tracked here and in `tasks.md` task 1.2 instead.

- PR 1, branch `test/memory-derived-waits` (change `memory-derived-waits`, kuru-memory test waits): its first commit, `test(memory): expose product budgets to test-support consumers`, adds `kuru_memory::test_budgets` under `test-support`, forwarding `OPERATION_TIMEOUT`, `QUERY_TIMEOUT`, `close_budget()` and `SUPERVISOR_REAP_ALLOWANCE` unchanged (no value change). Every `OPERATION_TIMEOUT` derivation in this change imports `kuru_memory::test_budgets::OPERATION_TIMEOUT`. Until PR 1 merges, exactly that one commit is cherry-picked onto this branch; after PR 1 merges, the orchestrator rebases onto main and the duplicate drops out.
  - Exposure commit sha: `3fa5318e` on `test/memory-derived-waits` (2026-10-03 23:19 -05:00), cherry-picked here as `93b8b35c`. It touches only `packages/kuru-memory/src/lib.rs`, `src/service/rpc.rs` (`OPERATION_TIMEOUT` becomes `pub(crate)`) and the new `src/test_budgets.rs`.
- PR #208 (`fix/instrumented-child-outlives-test`, open, another assistant) edits `apps/kuru-tui/tests/support/terminal.rs` lines 174-210 (the spawn ledger in `Terminal::spawn`, +15 lines). This change edits that file only in the constants block above `startup_timeout` (base lines 18-19, now 18-44: `FRAME_ALLOWANCE`, `READY_TIMEOUT`, `IO_TIMEOUT` and the `OPERATION_TIMEOUT` import) and in a comment above `wait_exit`'s drain; `Terminal::spawn` is untouched, so the hunks do not overlap and whichever merges second rebases (inference, not rehearsed). #208 also touches `kuru-memory` product files (`service.rs`, `server.rs`, `test_support.rs`) that this change does not edit.
