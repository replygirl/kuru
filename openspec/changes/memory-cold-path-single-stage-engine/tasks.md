# Tasks

## 1. Single-engine cold staging job

- [ ] 1.1 Replace `StageWorker::init`, `StageWorker::migrate` and
      `StageWorker::validate_and_mark` with one cold staging job that runs
      bootstrap, `initialize`, the optional legacy import,
      `migrations::upgrade`, `validate_active`, the revision read and
      `ready.json` (between the marker boundaries) on one engine start,
      modeled on `StageWorker::build_template`, and verify by reading the
      diff: one `Start` variant and one `self.start(...)` call cover the
      whole cold sequence, with no remaining call sites for the three
      removed methods.
- [ ] 1.2 Update the module doc comment at the top of `stage_worker.rs` to
      describe the new job list, and verify by reading it against the
      implementation.
- [ ] 1.3 Update `MemoryStore::create_cold` in `store.rs` to call the new
      job once and verify by reading the diff: quiescence, the rename and
      the active start in `open_inner` are unchanged.
- [ ] 1.4 Preserve failure-preservation and cancellation semantics: a
      failure before `ready.json` still calls `preserve_unready_stage` and
      leaves an unready stage; a cancelled opener still cannot release the
      startup lock before the reap. Verify by inspection against the
      existing `init`/`migrate`/`validate_and_mark` error-handling arms
      being folded into the one job without dropping a preservation path.

## 2. Tests

- [ ] 2.1 Add `cold_stage_initializes_migrates_and_validates_on_one_engine`
      (legacy import fixture): assert via `test_support::engine_ledger`
      that the cold open makes 2 engine starts and 1 owned close before
      ready, and verify it fails against the unmodified three-start code
      before task 1 lands (red/green).
- [ ] 2.2 Add `cold_stage_validation_failure_preserves_unready_stage`:
      inject a validation failure on the single cold engine and assert the
      stage is preserved under `interrupted/` without a second start, and
      that the project's active path stays absent.
- [ ] 2.3 Add `cancelled_cold_open_after_migration_reaps_before_lock_release`,
      in the pattern of
      `cancelled_open_during_stage_build_keeps_startup_lock_until_reap`:
      cancel the opener after the migration chain inside the single cold
      job and assert a second opener acquires the startup lock only after
      the engine ledger shows the reap.
- [ ] 2.4 Add `active_open_validates_after_reload`: assert a validation
      hook fires both on the cold job's stage engine and again on the
      active engine after the reload from disk, confirming the active
      start's `validate_active` call is untouched.
- [ ] 2.5 Retarget `store/open_pool_budget_tests.rs`: move the delayed-pool
      hook (`migrated_stage_pool_delay`) from the removed `Start::Validate`
      case to the new single cold job's pool, and verify
      `migrated_stage_pool_uses_remaining_startup_budget_and_post_open_pools_stay_ordinary`
      still asserts the same pool-timeout behavior against the one-start
      sequence.
- [ ] 2.6 Retarget the `Creation::Cold` migration-hook pause points in
      `store/recovery_tests.rs` (`process_loss_after_accepted_ddl_retains_attempt_until_cold_recovery`,
      `fresh_staging_process_loss_after_ddl_is_preserved_and_never_reused`
      and any other test pausing `MigrationRunnerHooks` mid-DDL during cold
      creation) and `store/migration_lifecycle_tests.rs` to the migration
      chain as it now runs inside the single cold job, without weakening
      what each test asserts about DDL acceptance and recovery.
- [ ] 2.7 Update `FreshOpen::Cold::starts()` in `test_support.rs` from 4 to
      2 (closes stays derived as `starts() - 1`) and its doc comment;
      verify `fresh_open_budget_of`, `fresh_open_budget` and
      `fixture_deadline` compile and their existing unit test
      (`test_support.rs` budget-ordering test) still holds with the new
      values.
- [ ] 2.8 Run the full `kuru-memory` test suite (`mise run //packages/kuru-memory:test`)
      and verify every retargeted and new test passes, and that
      `FreshOpen::Template` (2) and `FreshOpen::FirstProject` (3) stay
      unchanged.

## 3. Docs

- [ ] 3.1 Update `docs/memory.md` (the "four database starts" sentence
      describing the cold staged build) to two, and verify by re-reading
      the paragraph in context.
- [ ] 3.2 Update `apps/kuru-docs/concepts/memory.md` ("two database starts
      instead of four") to describe the cold path's new start count, and
      verify with `mise run docs:check`.
- [ ] 3.3 Update `docs/development.md`'s fixture-expectations section
      (`Creation::Cold` fixtures, legacy import and configured
      `dolt_binary` "keep the cold staged build's four starts";
      `fixture_deadline` "four starts per fresh open") to two, and verify
      the surrounding paragraph about `test_support::fresh_open_budget_of`
      and `FreshOpen` still reads correctly.
- [ ] 3.4 Confirm `openspec/specs/versioned-memory/spec.md` needs no edit:
      re-read "Store creation path selection" and "Current-schema staging
      and preserved failures" and verify neither names a cold-path start
      count that the collapse would falsify.

## 4. Benchmark

- [ ] 4.1 Run the in-process cold-fixture-open benchmark (legacy import) on
      this branch before task 1's change, record the baseline wall time
      (median of N≥5, same warm engine cache, same machine), then run it
      again after the change and record the after figure; verify by filling
      in `proposal.md`'s Benchmarks table with both measured numbers, N,
      and the machine/conditions, replacing the `[to measure]` placeholders.

## 5. Coverage and lint

- [ ] 5.1 Run `mise run //packages/kuru-memory:coverage` and verify the
      90% workspace line coverage gate still holds with the changed
      `stage_worker.rs` and `store.rs`.
- [ ] 5.2 Run `mise run lint ::: lint:windows ::: typecheck` and verify
      they pass clean, including the Windows-target lint pass over
      `stage_worker.rs`.
