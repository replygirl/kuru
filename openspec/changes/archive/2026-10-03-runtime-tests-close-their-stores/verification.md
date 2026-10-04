# Verification

## 1. No runtime test drops a live store [critical]

- [x] 1.1 @equivalence (agent) whole `kuru_runtime` lib (229 tests), uninstrumented, `KURU_TEST_DOLT_LOG_DIR` set, before the fix -> observed: 229 passed; 80 tests recorded `owner_dropped_live`, 110 dropped owners in total. 11 of the 80 are in partition 8's 35 names. The tests are in tests, review_tests, hook_tests, hook_platform_tests, mode_baseline_tests, notes_tests, preferences_tests, dolt_tests and engine::publication_tests.
- [x] 1.2 @regression (agent) `dreaming_rejects_last_role_removal…` with its teardown replaced by `drop(harness)` plus the assertion, then with `close_stores` -> observed: first FAILED "Dolt supervisor 1 for store … was dropped without a close, so its reaper thread, not the test, awaited its exit"; then passed
- [x] 1.3 @regression (agent) the same whole-lib trace after the fix, ×4 (one before guard scoping, three after) -> observed: 230 passed with 0 `owner_dropped_live` in runs 1, 3 and 4. Run 2 had 229 passed and 1 failed, with 1 `owner_dropped_live`: `hook_tests::cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants` panicked at its existing assertion "hook descendant survived dream cancellation" (a `sleep 30` survivor, ppid 1). That is before its teardown, so its store dropped while it unwound. This is a separate hook-reaping flake (cf. #201), outside this change's scope.
- [x] 1.4 @unit (agent) `close_stores_requires_every_store_the_test_opened` and `test_support::{engine_ledger,spawn_ledger}::tests` ×3 -> observed: 1 and 5 passed each run. The open store is reported as "has not been reaped" until it is closed, and the test-wide mark covers a supervisor started before a later mark.

## 2. No late profile [critical]

- [x] 2.1 @runtime (agent) partition 8's 35 runtime tests, instrumented (`-C instrument-coverage`), runner-like harness (own group, group SIGKILL, 60 s watch) with the trace, ×3 (built before the guard-scoping edit, which only moves assertions into blocks) -> observed: 35 passed each run; 51/49/49 profiles; 0 late; 0 `owner_dropped_live`; every supervisor `owner_finished`
- [x] 2.2 @regression (agent) `dreaming_rejects_last_role_removal…` alone, same harness with a 30 s watch, ×3 -> observed: 0 late profiles in 3/3. Before the fix it was late in 2 of 3 (change `instrumented-child-outlives-test`, 2.2).
- [~] 2.3 @runtime (agent) a native CI coverage shard -> defer: CI reruns are not allowed for this task; the PR's CI runs every partition

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `lint`, `lint:windows`, `typecheck`, `lint:tooling`, `docs:check`, `cospec -- validate --all --strict` -> observed: each exit 0. The first `lint` run flagged 7 std `MutexGuard`s held across the new teardown await (`await_holding_lock`). They are now scoped in blocks, and the rerun exited 0.
- [~] 3.2 @unit (agent) `mise run //packages/kuru-delivery:test` narrowed to `coverage::` -> defer: this change touches no kuru-delivery code; it passed ×3 at commit 7e643624 (change `instrumented-child-outlives-test`, 3.1)
