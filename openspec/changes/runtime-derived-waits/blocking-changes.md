# Dependencies

## Blocked by

None.

## Soft-blocked by

- [x] `runtime-cancelled-dream-join-wait` — provides `progress_wait::until_event`, `dream_gap_bound` and `HookHost::quiesce_bound()`, which the dream and join sites reuse. *(archived 2026-10-02)*
- [x] `runtime-post-cancel-joins` — provides `progress_wait::TaskWatch`, `join_on_progress`, `settle` and `unhooked_gap_bound`, and already fixed p1#24 and p1#2, so those two rows are dropped here. *(archived 2026-10-03)*

## Coordination, not a block

None of the items below is a change slug that exists on this branch, so cospec cannot ledger it (a dangling slug under either section above fails `cospec validate --strict` with `blockers/dangling-ref`, as the tui sibling observed). They are tracked here and in `tasks.md` task 1.2 instead.

- PR #210, branch `test/memory-derived-waits` (change `memory-derived-waits`, kuru-memory test waits): its commit `3fa5318e` (2026-10-03 23:19 -05:00, "test(memory): expose product budgets to test-support consumers") adds `kuru_memory::test_budgets` under `test-support`, forwarding `OPERATION_TIMEOUT`, `QUERY_TIMEOUT`, `close_budget()` and `SUPERVISOR_REAP_ALLOWANCE` unchanged (no value change). It touches only `packages/kuru-memory/src/lib.rs`, `src/service/rpc.rs` (`OPERATION_TIMEOUT` becomes `pub(crate)`) and the new `src/test_budgets.rs`.
  - Measured on origin/main cbebf7b7: `test_budgets` is absent from the tree, and `packages/kuru-runtime/Cargo.toml:24` already enables `kuru-memory`'s `test-support` in its dev-dependencies, so the cherry-pick alone is enough and no Cargo edit is needed.
  - Status at scoping: not cherry-picked. The implementer cherry-picks exactly that one commit before the first edit that imports `test_budgets`, records the new sha in task 1.2, and after #210 merges the orchestrator rebases onto main so the duplicate drops out.
  - Cherry-picked (2026-10-04): `3fa5318e` is `bf738381` on this branch, the only commit after cbebf7b7 that touches `packages/kuru-memory` (`git log --oneline cbebf7b7..HEAD -- packages/kuru-memory`). Drop it on rebase once #210 merges.
- PR #208, branch `fix/instrumented-child-outlives-test` (open, another assistant): its `fix(runtime): close every test's memory stores before it returns` commit edits `dolt_tests.rs`, `hook_tests.rs`, `hook_platform_tests.rs`, `review_tests.rs`, `tests.rs`, `mode_baseline_tests.rs`, `notes_tests.rs`, `preferences_tests.rs` and `engine.rs` (test module). Measured with `git diff -U0 origin/main...origin/fix/instrumented-child-outlives-test`: no hunk touches a site line in these files, and `accounting_tests.rs`, `dream.rs`, `permission_tests.rs` and `progress_tests.rs` are not edited by #208. The nearest hunks, in base-327f817c line numbers (equal to this tree's for every file #206 left alone):
  - `tests.rs`: a 36-line helper (`close_stores`) inserted after line 175, 17 lines below p1#49's :158 edit; every other `tests.rs` site (1032 to 1830) lies outside all #208 hunks.
  - `hook_tests.rs`: one inserted line after the end of the test that precedes p1#32's (base 1722, 9 lines above the script at :1731), and one inserted after the final `harness.shutdown(false)` of p1#32's test (base 1851, this tree's :1855), which sits immediately below p1#32's assertion at :1849-1854. The hunks are separated by one unchanged line, so the rebase should be mechanical (inference, not rehearsed); keep p1#32's edits out of the `harness.shutdown(false)` line.
  - `dolt_tests.rs`: an insertion after base 947, 59 lines below p1#19's :888. `review_tests.rs`: hunks after base 1461 and 2588, 76 and 60 lines from the nearest sites (:1385, :2528). `engine.rs`: one hunk at base 7435, far from :6786 to :6883.
  - #208 is based on 327f817c, so it must itself rebase over #206's `hook_tests.rs` hunks (:1752-1858); that is its owner's rebase, not this change's.
  - Edits here stay local to the listed lines so whichever PR merges second rebases mechanically.
  - #208 merged as 94cf53c5 after this branch's base. Measured 2026-10-04 with `git merge-tree --write-tree HEAD origin/main` (read-only): exit 0, no conflicted paths. The rebase onto it is not rehearsed and its tests are not run here.
- assistant6 owns the dream half of p1#23 (`accounting_tests.rs:3607` and `explain_expired_dream_wait`, :3654-3677). This change edits `accounting_tests.rs` only at :1783 (p1#22) and :3544 (the turn half of p1#23), 63 lines above assistant6's wait, and does not touch `progress_wait.rs`, `dream_gap_bound`, `quiesce_bound` or `turn_admission_deadline`, which assistant6 relies on unchanged.
- PR #215, branch `test/connectors-derived-waits` (kuru-connectors): not a dependency. Measured on its head 7d9c0e8e, `IO_TIMEOUT` is still `pub(crate)` at `lib.rs:74`, so this change restates it once instead of importing it.
