# Tasks

## 1. Reproduce first (regression tests red before any product edit)

- [x] 1.1 Add non-ignored kuru-memory regression tests that inject one lingering real server session on the candidate branch, then run product `delete_candidate_ref` on the promote path, and `cleanup_abandoned_candidate` (exclusion probe and `-D`) on the abandon path. Verify both FAIL on the unfixed tree with the exact `stage=cleanup … vendor=1105` record, and record the output (verification 1.1, 1.2). Landed in 34a1a9b4; reported red on the unfixed tree, green after.
- [x] 1.2 Add the Class A regression test: a `MemoryStore::open` error after server start, followed by a non-waiting lifecycle-lease attempt. Verify it FAILS on the unfixed tree (verification 2.1). Landed in 34a1a9b4 as `packages/kuru-memory/src/store/open_error_reap_tests.rs`.
- [x] 1.3 Add the fixture-directory invariant to `test_support` (non-waiting lease check, keep and name the directory on violation, no panic while unwinding). Verify it reports the unfixed `dream.rs` fixture, and record every other violating test it finds (verification 2.2). Landed in f5ec9c1b as `test_support::TempDir` / `fixture_dir.rs`; flagged and the fixtures lane fixed every call site it owns (`kuru-runtime`, `kuru-tui`).

## 2. H1: release sessions before a candidate ref is deleted or renamed

- [x] 2.1 Make candidate retirement end at server-observed session end (processlist absence on `kuru/<branch>`), bounded by the existing `QUERY_TIMEOUT` retirement deadline and tagged `PoolRetirement`. Place it per design D1, and use it before every delete, rename and exclusion probe. Verify tasks 1.1 turn green and the canary passes unmodified (verification 1.1, 1.2, 1.4). Landed in 34a1a9b4 as `retire_branch_sessions`, used by both the rename and delete paths.
- [x] 2.2 Add the never-ending-session test with a test-supplied deadline. Verify the `PoolRetirement` failure, that no ref changed, that no `--force` was used and that the fence trips (verification 1.3). Landed in 34a1a9b4 for the store-level step; the managed-client fence leg is not exercised end to end and stays open under task 7.3 and verification 1.3.
- [x] 2.3 Extend `candidate_branch_rename_reason` to label the exact in-use message at `Cleanup`. Verify only the diagnostic record changes (verification 1.5). Landed in 34a1a9b4 (`operational_gc_tests.rs`).

## 3. H2: never release a directory before its Dolt is reaped

- [x] 3.1 Make every `MemoryStore::open` error after `Server::open*` await the owned server's bounded close (`close_installed_guard`/`close_migration_worker`) and attach a close failure as context. Sites: store.rs 1724, 1770, 1803, 1878-1890, 1910-1914, 1938-1939 on c986f4ff. Verify task 1.2 turns green. Landed in 34a1a9b4.
- [x] 3.2 Add `test_support::await_managed_quiescence` (`retire_idle_service`, then `Server::quiescence_at` within `SUPERVISOR_REAP_ALLOWANCE`). Use it in `kuru-runtime/src/dream.rs` and in every managed fixture that task 1.3 reported. Verify the invariant passes across the full kuru-memory and kuru-runtime suites with no test excluded. Landed in f5ec9c1b; fixtures lane reports the kuru-runtime suite green at 220/220 (`--test-threads=2`). The kuru-memory suite was not yet rerun after 34a1a9b4 landed alongside it — full-suite confirmation: round 1 passed 325/0 at 220997cf, after fixing an invariant false positive in two ungated owner-lock tests.
- [x] 3.3 Re-run the 0.2 baseline loop and suites with tracing. Record the after-counts (verification 2.3). Round 1: 0 panics, 0 directories removed before exit and 0 unsuccessful supervisor exits across both loops and all traced suites (verification.md round 1 table).

## 4. Windows cancelled activation (split out of this change)

Not part of this change. The draft teardown was never type-checked or run on Windows and, by static reading, makes the frozen test `cancelling_checked_activation_recovery_drops_stage_before_cache_lock` fail deterministically (its own `_parent` handle keeps `runtime` delete-pending under kuru-platform's legacy delete disposition). It is parked, unmerged, as commit dbef9588 on branch `fix/memory-provision-stage-teardown` (worktree `tmp/worktrees/fix-memory-provision-stage-teardown`) and needs its own cospec change after a maintainer decision on that fixture handle. Catalogue items 1 and 6 are routed there, not fixed here. Implementer decision pending maintainer confirmation (review M1).

## 5. Documentation and verification

- [x] 5.1 Document the fixture teardown invariant and `await_managed_quiescence` in `docs/development.md`, and verify the docs checks pass. Landed in 171b1903 with the opt-in `measure:lifecycle` task; `docs:check` passed.
- [x] 5.2 Run the repository gates and the coverage gate, and record the results (verification 4.3, 4.4). Round 1: local repository gates and all four package suites pass (verification 4.3). Round 2: gates and suites pass again and local coverage is 94.30% (verification 4.4, round 2 table); the CI half stays with 5.3.
- [ ] 5.3 After push, collect the named macOS evidence set and the Ubuntu/Windows runs, and record every run and job id (verification 4.1, 4.2).
- [ ] 5.4 Update `tmp/roadmap` dx-followons §16 as superseded, and route items 13 and 14 plus the 10/11 deadline sub-items to the first-launch budget item (untracked notes, not part of the commit).

## 6. Review round 1 findings

- [x] 6.1 Hold `spawn_gate::locking_async` across the in-process lifecycle lease acquire and release in `open_error_reap_tests::assert_reaped`, and open every store in those tests through `spawn_gated_open`; no assertion changed and the fixture probe still never waits (review M2a). Landed in 545b0dc4.
- [x] 6.2 Decide the kuru-runtime/kuru-tui invariant exposure (review M2b). Round 2 recorded it as an accepted residual; that decision was rejected and replaced by task 7.1, which removes the exposure.
- [x] 6.3 Point the canary at `retire_branch_sessions` with its assertions byte-identical (review S6, design D1 fallback). Landed in cc17e7a3.
- [x] 6.4 Replace the copied `MANAGED_REAP_ALLOWANCE` with `server::SUPERVISOR_REAP_ALLOWANCE` (review S2; value unchanged, 13 s). Landed in 0f944821.
- [x] 6.5 Add a test that a failing close is attached to the original open error, and cover the final-validation open arm (review S1, verification 2.4). Landed in 545b0dc4.
- [x] 6.6 Park the uncommitted Windows draft on its own branch and restore this worktree to the verified head (review S4).
- [x] 6.7 Fix the Class B kuru-tui fixture the round-2 traces found (`notice_text_never_reaches_the_provider_request_for_a_real_tui_turn`) by closing its store before the root drops, and guard its root (verification 2.6). Landed in 13e81e31.

## 7. Round 3 hardening (review M2 residual rejected)

- [x] 7.1 Replace the fixture guard's non-waiting flock probe with process-local quiescence records (`test_support/engine_ledger.rs`): in-process supervisors stay live until this process reaps them and that reap records the store; `await_store_quiescence` records while it holds the lifecycle lease; `await_managed_quiescence` uses it for the project store and its staging siblings. Add the deterministic tests for an unrecorded store, a duplicated lease descriptor after awaited quiescence, a panicking test, an unreaped owner and a stale record (verification 2.5).
- [x] 7.2 Move memory data roots under plain `tempfile` directories in consumer test code to the guarded root (review N9): kuru-runtime `accounting_tests::abandoned_dream_keeps_usage_after_reopen_without_advancing_main` and `review_tests` (failed MCP fixture), the kuru-tui integration `ServiceCleanup` roots (14 call sites) and the packaged `embedded_runtime` acceptance root (acceptance passed locally).
- [x] 7.3 Exercise the managed-client fence for the session-wait timeout end to end (verification 1.3). Landed as the slow (about 31 s) ordinary-suite test `store::operational_gc_tests::slow_30s_managed_abandon_cleanup_bound_fences_client_and_keeps_status_ref`: a managed client meets the constant `QUERY_TIMEOUT` through the service with no product change; see verification 1.3.
- [x] 7.4 Restate design D5, verification 2.5 and `docs/development.md` for the record-based invariant.

## 8. CI round 1 (run 36403054163 at 61b17e9f): invariant fixes

- [x] 8.1 Stop a recycled Linux inode from inheriting a removed store's record (Ubuntu `a_store_without_a_quiescence_record_keeps_the_root_and_names_the_test`): releasing a root forgets the records beneath it (`releasing_a_root_forgets_the_records_beneath_it`), and keys add the directory's birth time for stores removed another way (macOS reproducer `a_record_for_an_earlier_directory_with_the_same_identity_explains_nothing`). Join the fixture's store path by component for Windows (verification 2.5, round 4).
- [x] 8.2 Replace the Windows `identity.json`-only store discovery with the external lease rule, on every platform: an `identity.json` directory is a store only when its `lifecycles/<identity>.lock` exists (templates and unopened copies were flagged). Add `an_identified_directory_is_a_store_only_under_its_external_lease`. Await managed quiescence in the two Windows service fixtures whose owners ran in other processes.
- [x] 8.3 Run the ledger and the whole teardown scan in one critical section. Round 1 gave as its reason that the scan loop's non-atomic coverage counters lost concurrent updates (the Windows coverage partitions refused a negative counter expression in `fixture_dir::collect`). That reason was wrong: the instrumented increments are atomic, and the refusal recurred (9.3). The section stays because it keeps a teardown's verdict and its forgetting of records consistent. No orchestrator change.

## 9. CI round 2 (run 36413703472 at c262f4f0): teardown threads

- [x] 9.1 Release the Windows owner fixture root on the test thread. `test_support::release_after_creator_exit` waits for the creator within the fixture's existing 40 s, then releases the root through the guard or keeps it, before it returns. `tests/windows_lifecycle.rs` `Fixture::drop` uses it and keeps its creator handle only when the wait ends without an exit, with both of its existing diagnostics. Add structural tests on every platform (verification 2.5, round 5).
- [x] 9.2 Record a dropped owner's reap on the ledger's next reader. Under test-support only, the reaper thread sends one report built from standard-library calls, and `engine_ledger::with` records pending reports first, as of the reap; a record never replaces a later one. The product reaper is unchanged.
- [x] 9.3 Correct the negative-counter explanation (8.3, verification round 4, `engine_ledger.rs`, `fixture_dir.rs`, `docs/development.md`): the exit-time profile write caught a detached, never-joined fixture thread inside the guard's scan, so a counter expression went negative. No update was lost.
- [ ] 9.4 CI: all 8 windows-latest coverage partitions pass the line export on two consecutive runs, with no negative counter expression for any function, and `windows_lifecycle` passes 12/12 with no "cleanup is delayed" or "cleanup is unproven" lines. Run 36426742472 attempt 2 (verification round 6): every Windows job green, including windows-latest coverage partitions 2 and 8; one of the two consecutive runs. The `windows_lifecycle` log lines were not re-read for this record.

## 10. CI round 3 (run 36426742472 attempt 2, job 108957163635): served-owner fixture teardown

- [x] 10.1 Retire an in-process `ServiceOwner` on every exit path of the fixtures that serve one (diagnosis `$S/flake/ci/pr125-run3b/diagnosis.md`). Add `test_support::ServedOwner` (a slot that empties once its task has ended, so no finished owner is polled twice) and `test_support::settle` (prints a body error to the captured output, then runs the teardown on every path; the body error wins, a teardown failure is attached as context). Apply it to all 22 fixtures the diagnosis lists, with each fixture's success-path calls and bounds; owner tasks are no longer aborted on drop inside the body. Guard, ledger, product code and assertions unchanged (verification round 6).
- [x] 10.2 Red before and green after: the diagnosis's injected early `bail!` in `managed_session_lifecycle_is_reversible_receipted_and_candidate_isolated` panics at `fixture_dir.rs:129` with the CI message before the fix and fails with its own error, no guard panic, after; the new `served_owner` tests are red with the teardown skipped (verification round 6).
- [x] 10.3 Document the served-owner rule in `docs/development.md` and design D5.
- [ ] 10.4 CI: the next run names the real error if `managed_session_lifecycle_…` fails again on ubuntu-24.04-arm; the hidden error of attempt 2 stays unknown.
