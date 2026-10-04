# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Coordination, not a block

Integration update (2026-10-04): PR #210 merged as `2d4b5b8e`; its memory
budget exports now come from main, and the duplicate `93b8b35c` cherry-pick
was dropped. PR #208 is also on main. PR #221 merged as `22e1ee60`; the TUI
wait changes were rebased onto it, retaining the closing wrappers, explicit
store closes and every prior source change. PR #214 and PR #215 remain the
unmerged prerequisites in the sequencing below. The earlier coordination
record follows for provenance.

No item below is a change slug that exists on this branch, so cospec cannot ledger it (a dangling slug fails `cospec validate --strict`: `blockers/dangling-ref`, observed when `memory-derived-waits` was tried under `Soft-blocked by`). Each is tracked here instead, and in `tasks.md` (task 1.2 for PR 1, task 7.1 for PR #214).

- PR 1, branch `test/memory-derived-waits` (change `memory-derived-waits`, kuru-memory test waits): its first commit, `test(memory): expose product budgets to test-support consumers`, adds `kuru_memory::test_budgets` under `test-support`, forwarding `OPERATION_TIMEOUT`, `QUERY_TIMEOUT`, `close_budget()` and `SUPERVISOR_REAP_ALLOWANCE` unchanged (no value change). Every `OPERATION_TIMEOUT` derivation in this change imports `kuru_memory::test_budgets::OPERATION_TIMEOUT`. Until PR 1 merges, exactly that one commit is cherry-picked onto this branch; after PR 1 merges, the orchestrator rebases onto main and the duplicate drops out.
  - Exposure commit sha: `3fa5318e` on `test/memory-derived-waits` (2026-10-03 23:19 -05:00), cherry-picked here as `93b8b35c`. It touches only `packages/kuru-memory/src/lib.rs`, `src/service/rpc.rs` (`OPERATION_TIMEOUT` becomes `pub(crate)`) and the new `src/test_budgets.rs`.
- PR #208 (`fix/instrumented-child-outlives-test`, open, another assistant) edits `apps/kuru-tui/tests/support/terminal.rs` lines 174-210 (the spawn ledger in `Terminal::spawn`, +15 lines). This change edits that file only in the constants block above `startup_timeout` (base lines 18-19, now 18-44: `FRAME_ALLOWANCE`, `READY_TIMEOUT`, `IO_TIMEOUT` and the `OPERATION_TIMEOUT` import) and in a comment above `wait_exit`'s drain; `Terminal::spawn` is untouched, so the hunks do not overlap and whichever merges second rebases (inference, not rehearsed). #208 also touches `kuru-memory` product files (`service.rs`, `server.rs`, `test_support.rs`) that this change does not edit.
- PR #214, branch `test/memory-open-hold-budget` (change `memory-open-hold-budget`, open): this change adopts its hand-off contract for the test-support open hold (kuru-memory `service/activity.rs`, `hold`, documented at `OPEN_HOLD_DIR_ENV`). Each `<Stage>.hold` marker this change writes carries, as ASCII decimal milliseconds, the budget of the test wait that ends in its removal, written before the owner is spawned (task 7.1). On this branch's seam `hold` checks only that the marker exists and never reads its content, so the adoption is inert and safe before #214 merges; the CI failure it answers (run 37180709856, task 7.1) is fixed only once #214 is on main and this branch is rebased onto it. #214 edits only `service/activity.rs`, `docs/development.md` and its own openspec directory, none of which this change edits.
- Merge order: #212 (this change) merges after #214 and after PR #215 (`test/connectors-derived-waits`, change `connectors-derived-waits`, kuru-connectors test waits). #215 shares no file with this change (its file list is kuru-connectors and its own openspec directory); the order is the orchestrator's sequencing of the priority subset.
