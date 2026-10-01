# Verification

## 1. Cold staged build runs on one engine instead of three [critical]

- [ ] 1.1 @unit (agent) run `cold_stage_initializes_migrates_and_validates_on_one_engine` (legacy import via `Creation::Cold`, counted through `test_support::engine_ledger`) -> ledger shows 2 engine starts and 1 owned close before ready, down from 4/3
- [ ] 1.2 @benchmark (agent) in-process cold-fixture-open benchmark (legacy import fixture), median of N>=5 on the same machine and warm engine cache, before and after the change -> after wall time is lower than before; both numbers and N recorded in proposal.md's Benchmarks table (tasks.md 4.1)
- [ ] 1.3 @integration (agent) run `mise run //packages/kuru-memory:test` for the full kuru-memory suite after the collapse -> all tests pass, including the retargeted open_pool_budget_tests.rs, recovery_tests.rs and migration_lifecycle_tests.rs cases

## 2. Observable behavior is unchanged (perf covenant)

- [ ] 2.1 @equivalence (agent) run the existing fixture, recovery and marker-boundary suites (store/recovery_tests.rs, marker_fixture-based tests, store/migration_lifecycle_tests.rs) against the collapsed cold path -> same pass/fail shape as before the change; no test's asserted behavior weakened, only its hook attachment point retargeted
- [ ] 2.2 @unit (agent) run `cold_stage_validation_failure_preserves_unready_stage` (inject a validation failure on the single cold engine) -> the stage is preserved under interrupted/ without a second engine start, and no store appears at the project's active path, matching today's three-start failure outcome
- [ ] 2.3 @unit (agent) run `cancelled_cold_open_after_migration_reaps_before_lock_release`, in the pattern of cancelled_open_during_stage_build_keeps_startup_lock_until_reap -> a second opener acquires the project's startup lock only after the engine ledger shows the cancelled cold job's engine reaped
- [ ] 2.4 @unit (agent) run `active_open_validates_after_reload` -> a validation hook fires on the cold job's stage engine and again on the active engine after the reload from disk, confirming validate_active still runs twice as it does today
- [ ] 2.5 @regression (agent) confirm FreshOpen::Template (2 starts) and FreshOpen::FirstProject (3 starts) budgets and their covering tests are untouched by this change -> only FreshOpen::Cold (4 -> 2) moves; template and first-project call sites unchanged
- [ ] 2.6 @manual (human) re-read versioned-memory spec.md's "Store creation path selection" and "Current-schema staging and preserved failures" requirements after the change -> both still read true with no edit needed, confirming this ships as perf with no requirement-text delta
