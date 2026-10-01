# Tasks

> Evidence below citing base `46c2d60c` and head `ef86dc74` was recorded by
> the prior implementing session before this branch was rebased onto
> `origin/main` (one upstream commit, `70fa6c0a`). The rebase was mechanical
> (no conflicts) and touched neither `stage_worker.rs` nor `store.rs` beyond
> context; the PR's actual commits are `099ee396`, `5fd94113` and
> `1244b41e`. CI on the rebased head `1244b41e` (run `36863567899`, green
> after one stated rerun of three unrelated Windows coverage partitions) is
> the first independent re-run confirmation of that evidence; see task 5.1.

## 1. Single-engine cold staging job

- [x] 1.1 Replace `StageWorker::init`, `StageWorker::migrate` and
      `StageWorker::validate_and_mark` with one cold staging job that runs
      bootstrap, `initialize`, the optional legacy import,
      `migrations::upgrade`, `validate_active`, the revision read and
      `ready.json` (between the marker boundaries) on one engine start,
      modeled on `StageWorker::build_template`, and verify by reading the
      diff: one `Start` variant and one `self.start(...)` call cover the
      whole cold sequence, with no remaining call sites for the three
      removed methods.
      Evidence: `StageWorker::build_cold` makes the one `self.start(..,
      Start::Cold, ..)` call and hands the server to a spawned
      `StageSession`; `Start` is now `Cold`, `Adopt`, `TemplateBuild`;
      `build_schema` is shared with `build_template`; `grep` finds no
      `init`/`migrate`/`validate_and_mark` call sites (commit `ef86dc74`).
- [x] 1.2 Update the module doc comment at the top of `stage_worker.rs` to
      describe the new job list, and verify by reading it against the
      implementation.
      Evidence: module doc lists `build_cold`, `adopt_and_mark` and
      `build_template` and the worker's ownership from the hand-over.
- [x] 1.3 Update `MemoryStore::create_cold` in `store.rs` to call the new
      job once and verify by reading the diff: quiescence, the rename and
      the active start in `open_inner` are unchanged.
      Evidence: `create_cold` calls `worker.build_cold(..)` once, then
      `Server::quiescence_at`; `open_inner` changes only the `Arc` around the
      legacy import and a `#[cfg(test)]` probe call before the active
      `validate_active`.
- [x] 1.4 Preserve failure-preservation and cancellation semantics: a
      failure before `ready.json` still calls `preserve_unready_stage` and
      leaves an unready stage; a cancelled opener still cannot release the
      startup lock before the reap. Verify by inspection against the
      existing `init`/`migrate`/`validate_and_mark` error-handling arms
      being folded into the one job without dropping a preservation path.
      Evidence: `StageSession::finish` preserves when the close returned the
      lock and `ready.json` is absent (the union of the three old arms) and
      keeps each phase's preservation and cleanup text; tests 2.2 and 2.3
      pass. Changed in effect, as intended by the design: a cancelled opener
      now leaves the worker to finish the whole session (see proposal).

## 2. Tests

- [x] 2.1 Add `cold_stage_initializes_migrates_and_validates_on_one_engine`
      (legacy import fixture): assert via `test_support::engine_ledger`
      that the cold open makes 2 engine starts and 1 owned close before
      ready, and verify it fails against the unmodified three-start code
      before task 1 lands (red/green).
      Evidence: on base `46c2d60c` it failed with "the stage was marked
      ready after 3 engine starts"; on head it passes for `Creation::Cold`
      and a legacy import (2 starts each, 1 start live at the ready marker)
      and for the existing-project reopen (1 start).
- [x] 2.2 Add `cold_stage_validation_failure_preserves_unready_stage`:
      inject a validation failure on the single cold engine and assert the
      stage is preserved under `interrupted/` without a second start, and
      that the project's active path stays absent.
      Evidence: passes; the open fails with "uncommitted changes" alone,
      1 start, no live engine, one preserved stage without `ready.json`.
- [x] 2.3 Add `cancelled_cold_open_after_migration_reaps_before_lock_release`,
      in the pattern of
      `cancelled_open_during_stage_build_keeps_startup_lock_until_reap`:
      cancel the opener after the migration chain inside the single cold
      job and assert a second opener acquires the startup lock only after
      the engine ledger shows the reap.
      Evidence: passes; the lock is held while the worker is paused at the
      boundary before `ready.json`, a second opener acquires it with no live
      engine, and the next open reuses the ready stage.
- [x] 2.4 Add `active_open_validates_after_reload`: assert a validation
      hook fires both on the cold job's stage engine and again on the
      active engine after the reload from disk, confirming the active
      start's `validate_active` call is untouched.
      Evidence: passes; the probe records the `.staging-` directory and then
      the active directory with the same native identity, 2 starts.
- [x] 2.5 Retarget `store/open_pool_budget_tests.rs`: move the delayed-pool
      hook (`migrated_stage_pool_delay`) from the removed `Start::Validate`
      case to the new single cold job's pool, and verify
      `migrated_stage_pool_uses_remaining_startup_budget_and_post_open_pools_stay_ordinary`
      still asserts the same pool-timeout behavior against the one-start
      sequence.
      Evidence: renamed `stage_pool_delay` and
      `stage_pool_uses_remaining_startup_budget_and_post_open_pools_stay_ordinary`;
      same assertions, passes; `staged_open_error_returns_only_after_its_server_is_reaped`
      now asserts "open staged main pool" and passes.
- [x] 2.6 Retarget the `Creation::Cold` migration-hook pause points in
      `store/recovery_tests.rs` (`process_loss_after_accepted_ddl_retains_attempt_until_cold_recovery`,
      `fresh_staging_process_loss_after_ddl_is_preserved_and_never_reused`
      and any other test pausing `MigrationRunnerHooks` mid-DDL during cold
      creation) and `store/migration_lifecycle_tests.rs` to the migration
      chain as it now runs inside the single cold job, without weakening
      what each test asserts about DDL acceptance and recovery.
      Evidence: no change needed (the hooks pause the same chain inside the
      one session); both files pass unchanged in the full suite. The
      accepted-DDL case of
      `cancelled_open_during_stage_build_keeps_startup_lock_until_reap`
      now expects the worker to finish and the next open to reuse the ready
      stage.
- [x] 2.7 Update `FreshOpen::Cold::starts()` in `test_support.rs` from 4 to
      2 (closes stays derived as `starts() - 1`) and its doc comment;
      verify `fresh_open_budget_of`, `fresh_open_budget` and
      `fixture_deadline` compile and their existing unit test
      (`test_support.rs` budget-ordering test) still holds with the new
      values.
      Evidence: `fresh_open_budgets_follow_each_creation_path` and
      `single_stall_defaults_match_the_reviewed_bounds` (126, 158, 222,
      318 s) pass; `fresh_open_budget` is the first project's;
      `template_warm_up_bound` keeps four starts and three closes.
- [x] 2.8 Run the full `kuru-memory` test suite (`mise run //packages/kuru-memory:test`)
      and verify every retargeted and new test passes, and that
      `FreshOpen::Template` (2) and `FreshOpen::FirstProject` (3) stay
      unchanged.
      Evidence: exit 0; lib 529 passed, 0 failed, 4 ignored; every
      integration target passed.

## 3. Docs

- [x] 3.1 Update `docs/memory.md` (the "four database starts" sentence
      describing the cold staged build) to two, and verify by re-reading
      the paragraph in context.
- [x] 3.2 Update `apps/kuru-docs/concepts/memory.md` ("two database starts
      instead of four") to describe the cold path's new start count, and
      verify with `mise run docs:check`.
      Evidence: `docs:check` passed ("Public docs artifacts, local links and
      anchors passed").
- [x] 3.3 Update `docs/development.md`'s fixture-expectations section
      (`Creation::Cold` fixtures, legacy import and configured
      `dolt_binary` "keep the cold staged build's four starts";
      `fixture_deadline` "four starts per fresh open") to two, and verify
      the surrounding paragraph about `test_support::fresh_open_budget_of`
      and `FreshOpen` still reads correctly.
- [x] 3.4 Confirm `openspec/specs/versioned-memory/spec.md` needs no edit:
      re-read "Store creation path selection" and "Current-schema staging
      and preserved failures" and verify neither names a cold-path start
      count that the collapse would falsify.
      Evidence: neither names a count; "the cold staged build keeps its own
      engine starts" and the owned live staging session scenario stay true.

## 4. Benchmark

- [x] 4.1 Run the in-process cold-fixture-open benchmark (legacy import) on
      this branch before task 1's change, record the baseline wall time
      (median of N≥5, same warm engine cache, same machine), then run it
      again after the change and record the after figure; verify by filling
      in `proposal.md`'s Benchmarks table with both measured numbers, N,
      and the machine/conditions, replacing the `[to measure]` placeholders.
      Evidence: proposal Benchmarks; interleaved N=10 each: cold p50
      3099 → 2409 ms, legacy p50 3129 → 2409 ms.

## 5. Coverage and lint

- [x] 5.1 Run `mise run //packages/kuru-memory:coverage` and verify the
      90% workspace line coverage gate still holds with the changed
      `stage_worker.rs` and `store.rs`.
      Not run locally: no package-scoped coverage task exists; the root
      `coverage` task instruments every package's suite, which the shared,
      disk-limited host could not take alongside other builds.
      Evidence: CI run `36863567899` at head `1244b41e` (green after one
      stated rerun of three unrelated Windows coverage partitions), merge
      job logs — macOS 94.64% (113283/119694 lines), Ubuntu 94.65%
      (113199/119585 lines), Windows 93.58% (114496/122347 lines), all
      against the 90% gate; the "Require native coverage and installation
      checks" gate job passed on each OS.
- [x] 5.2 Run `mise run lint ::: lint:windows ::: typecheck` and verify
      they pass clean, including the Windows-target lint pass over
      `stage_worker.rs`.
      Evidence: `//packages/kuru-memory:lint`, `:lint:windows` and
      `:typecheck` exit 0; `format:check` passed.
