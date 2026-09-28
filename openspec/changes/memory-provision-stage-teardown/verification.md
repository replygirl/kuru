# Verification

## 1. The installation lock outlives an unresolved stage on no path [critical]

- [ ] 1.1 @regression (agent) Unix `provision::tests::failed_extraction_receipts_a_refused_stage_before_releasing_the_lock` on unfixed and fixed code -> FAILS before (no retention named, no receipt), PASSES after with the diagnostic observed while the lock is held
- [ ] 1.2 @regression (agent) Unix `provision::tests::cancelled_extraction_receipts_a_refused_stage_before_releasing_the_lock` on unfixed and fixed code -> FAILS before (no receipt when the lock becomes free), PASSES after
- [ ] 1.3 @regression (agent) Unix `provision::tests::published_stage_retention_is_receipted_before_the_lock_is_released` on unfixed and fixed code -> FAILS before (lock already free at the diagnostic), PASSES after
- [ ] 1.4 @regression (agent) Windows `provision::native_tests::cancelled_activation_recovery_receipts_a_held_stage_before_releasing_the_lock` -> PASSES in the windows-latest and windows-11-arm partitions that list it

## 2. Windows activation recovery boundary [critical]

- [ ] 2.1 @regression (agent) Windows `provision::native_tests::first_checked_no_move_after_the_window_reports_stopped_recovery` -> PASSES in the windows partitions (red on unfixed code by construction: bare first error)
- [ ] 2.2 @regression (agent) Windows `provision::native_tests::cancellation_at_a_late_first_checked_no_move_is_cancelled` -> PASSES in the windows partitions (red on unfixed code by construction: `Ok(Err)`, the job 108878373677 panic)
- [ ] 2.3 @unit (agent) frozen tests `persistent_held_descendant_exhausts_checked_recovery_and_preserves_stage` and `cancelling_checked_activation_recovery_drops_stage_before_cache_lock` -> PASS with unchanged assertions in the windows partitions

## 3. Unchanged behavior and gates

- [ ] 3.1 @unit (agent) provision test modules at `--test-threads=2` (including the existing cancellation, probe, receipt and sweep tests) -> all pass
- [ ] 3.2 @integration (agent) `mise run //packages/kuru-memory:test`, `format:check`, `lint`, `typecheck`, `lint:tooling`, `docs:check`, `cospec validate --strict` -> all exit 0

## 4. Catalogue dispositions

- [ ] 4.1 @manual (agent) item 6 (M1, job 108544879759), item 1 (M2, job 108484586358) and the PR #124 occurrence (M2, job 108878373677) -> each mapped to its mechanism and regression test here and in `flaky-tests.md`
