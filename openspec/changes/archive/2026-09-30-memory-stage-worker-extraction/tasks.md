# Tasks

## 1. Stage worker module

- [x] 1.1 Create `packages/kuru-memory/src/store/stage_worker.rs` with a
      `StageWorker` type holding the inputs every job needs (the
      `ServerOptions`-building closure, staging `PathBuf`, `parent`,
      `lifecycle_root`, `timeout`, `project_scope`, legacy import), and
      declare the module from `store.rs` and verify `cargo check -p
      kuru-memory` compiles with the new empty module wired in.
- [x] 1.2 Move today's init step (staging directory creation, start 1,
      `initialize` + optional `import`, `close_migration_worker`,
      `preserve_unready_stage` on failure) into `StageWorker::init`
      verbatim, including its exact `.context(...)` text and its
      `(Ok, Ok) / (Err, Ok) / (Ok, Err) / (Err, Err)` match arms, and verify
      by diffing the moved block against the pre-move source (no text
      changes beyond `self`/module-path adjustments).
- [x] 1.3 Move today's migrate step (reopen, start 2, call to
      `run_migration_worker`, `preserve_unready_stage` on failure) into
      `StageWorker::migrate` verbatim, keeping `run_migration_worker` itself
      in `store.rs`, and verify by the same diff check as 1.2.
- [x] 1.4 Move today's validate-and-mark step (reopen, start 3,
      `validate_active`, `revision`, `Activation` construction, the
      `marker_fixture::reach` Before/After calls around the `ready.json`
      write, `close_migration_worker`, the `ready.json`-exists-gated
      `preserve_unready_stage`) into `StageWorker::validate_and_mark`
      verbatim, and verify by the same diff check as 1.2.
- [x] 1.5 Rewrite `open_inner`'s fresh-store branch to construct a
      `StageWorker` and call `init`, `migrate`, `validate_and_mark` in
      sequence, threading the startup lock exactly as today
      (`lock.take()` in, `lock = Some(returned)` out between calls), leaving
      `recover_staging`, the move-to-active quiescence/rename, and the
      active-path open untouched, and verify `cargo check -p kuru-memory`
      compiles and `rg "fn open_inner"` shows the fresh-store branch body
      shrunk to the worker calls plus the unchanged rename/active-open tail.

## 2. Creation selector

- [x] 2.1 Add `enum Creation { Default, Cold }` (both variants constructed
      explicitly, no derived `Default`) and a `creation: Creation` field on `OpenOptions`, gated
      `cfg(any(test, feature = "test-support"))`, defaulted in
      `OpenOptions::new`, and verify `cargo check -p kuru-memory --features
      test-support` compiles.
- [x] 2.2 Set `Creation::Cold` in `MemoryStore::temporary_cold` and leave
      `temporary()` / `open_temporary()` on the default, and verify the 15
      existing call sites of `temporary()`, `temporary_cold()`,
      `open_temporary()` (per `tmp/roadmap/store-creation-research/A-open-inner-and-recovery.md`
      and a grep of `packages/` and `apps/`) still compile unchanged.
- [x] 2.3 Grep `packages/` and `apps/` for `OpenOptions {` struct-literal
      construction outside `packages/kuru-memory/src/store.rs` and
      `test_support.rs`, and confirm none exists, recording the grep command
      and its empty result as acceptance evidence that no product caller can
      set `Creation`.

## 3. Cancellation test

- [x] 3.1 Add
      `cancelled_open_during_stage_build_keeps_startup_lock_until_reap` in
      `packages/kuru-memory/src/store/stage_worker.rs`'s test module (the
      store tests reach it through the same private items): start an opener,
      cancel it mid `StageWorker` build (using the existing
      `marker_fixture`/hook mechanism the way other in-stage-build tests
      pause), assert a second opener's startup-lock acquisition completes
      only after the test-only `engine_ledger` records the first engine's
      reap, and verify the test passes under `mise run
      //packages/kuru-memory:test` and fails (by temporarily reverting the
      lock hand-off) if the lock were released before the reap.

## 4. No-behaviour-change verification

- [x] 4.1 Run the existing marker-boundary, recovery (`recover_staging`,
      Class R/U-equivalent today's cases), and preservation
      (`preserve_unready_stage`) tests unchanged and record their pass
      under `mise run //packages/kuru-memory:test`, confirming no assertion
      needed updating.
- [x] 4.2 Confirm the fresh-open engine-start/close/process-launch counts
      are unchanged (4 starts, 3 closes, 10 process launches) by running the
      existing harness/measurement test(s) that assert those counts (per
      `tmp/roadmap/store-creation-research/A-open-inner-and-recovery.md` and
      any `engine_ledger`-based count assertion in `kuru-memory`'s test
      suite) and recording the observed counts as acceptance evidence.
      Outcome: no kuru-memory test asserts exact start, close or launch
      counts, and no new measurement is in scope; the counts are shown
      unchanged by construction (verification 1.2).
- [x] 4.3 Run `mise run //packages/kuru-memory:lint` and `mise run
      //packages/kuru-memory:typecheck` (or the workspace-level
      equivalents) and record a clean pass, including for the Windows lint
      target.
- [x] 4.4 Update `docs/development.md` only if it names
      `run_migration_worker`, `close_migration_worker`,
      `preserve_unready_stage`, or `open_inner`'s staging steps by path;
      otherwise record that no doc change was needed and why.
      Outcome: `grep -rn "run_migration_worker\|close_migration_worker\|preserve_unready_stage\|open_inner\|close_failed_open\|stage_worker" docs apps/kuru-docs --include=*.md`
      matched nothing; `docs/development.md:379` names only
      `MemoryStore::temporary_cold()`, whose signature is unchanged, so no
      documentation change is needed.
