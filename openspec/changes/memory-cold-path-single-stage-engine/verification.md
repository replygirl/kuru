# Verification

## 1. Cold staged build runs on one engine instead of three [critical]

- [x] 1.1 @unit (agent) run `cold_stage_initializes_migrates_and_validates_on_one_engine` (legacy import and `Creation::Cold`, counted through `test_support::engine_ledger`) -> observed: base `46c2d60c` failed with 3 starts at the ready marker; head `ef86dc74` passes with 1 start live at the marker, 2 starts per creation, 1 for the existing-project reopen
- [x] 1.2 @benchmark (agent) in-process cold-fixture-open timing (cold and legacy import), base and head executables run alternately, N=10 each -> observed: cold p50 3099 -> 2409 ms, legacy p50 3129 -> 2409 ms, 4 -> 2 starts in every run; a non-interleaved pair was within noise; numbers and conditions in proposal.md's Benchmarks
- [x] 1.3 @integration (agent) run `mise run //packages/kuru-memory:test` for the full kuru-memory suite after the collapse -> observed: exit 0, lib 529 passed 0 failed 4 ignored, every integration target passed

## 2. Observable behavior is unchanged (perf covenant)

- [x] 2.1 @equivalence (agent) run the existing fixture, recovery and marker-boundary suites (store/recovery_tests.rs, marker_fixture-based tests, store/migration_lifecycle_tests.rs) against the collapsed cold path -> observed: all pass in the full suite with no change to recovery_tests.rs or migration_lifecycle_tests.rs; the one changed expectation is the accepted-DDL cancellation case, where the worker now finishes and the next open reuses the ready stage, as the design's cancellation table requires
- [x] 2.2 @unit (agent) run `cold_stage_validation_failure_preserves_unready_stage` (inject a validation failure on the single cold engine) -> observed: open fails with "uncommitted changes", 1 start, no live engine, one unready stage under interrupted/, no active directory
- [x] 2.3 @unit (agent) run `cancelled_cold_open_after_migration_reaps_before_lock_release` -> observed: lock held while the worker is paused, a second opener acquires it only with no live engine, next open reuses the ready stage
- [x] 2.4 @unit (agent) run `active_open_validates_after_reload` -> observed: the probe records the stage directory and then the active directory, same native identity, 2 starts
- [x] 2.5 @regression (agent) confirm FreshOpen::Template (2 starts) and FreshOpen::FirstProject (3 starts) budgets and their covering tests are untouched by this change -> observed: `fresh_open_budgets_follow_each_creation_path` asserts (2,1), (3,2), (2,1); template open_tests pass; release harness counts first-launch 3, new-project 2, cold-existing 1 on base and head
- [ ] 2.6 @manual (human) re-read versioned-memory spec.md's "Store creation path selection" and "Current-schema staging and preserved failures" requirements after the change -> both still read true with no edit needed, confirming this ships as perf with no requirement-text delta
