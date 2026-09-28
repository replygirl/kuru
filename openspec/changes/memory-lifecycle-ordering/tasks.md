# Tasks

## 1. Reproduce first (regression tests red before any product edit)

- [ ] 1.1 Add non-ignored kuru-memory regression tests that inject one lingering real server session on the candidate branch, then run product `delete_candidate_ref` on the promote path, and `cleanup_abandoned_candidate` (exclusion probe and `-D`) on the abandon path. Verify both FAIL on the unfixed tree with the exact `stage=cleanup … vendor=1105` record, and record the output (verification 1.1, 1.2).
- [ ] 1.2 Add the Class A regression test: a `MemoryStore::open` error after server start, followed by a non-waiting lifecycle-lease attempt. Verify it FAILS on the unfixed tree (verification 2.1).
- [ ] 1.3 Add the fixture-directory invariant to `test_support` (non-waiting lease check, keep and name the directory on violation, no panic while unwinding). Verify it reports the unfixed `dream.rs` fixture, and record every other violating test it finds (verification 2.2).

## 2. H1: release sessions before a candidate ref is deleted or renamed

- [ ] 2.1 Make candidate retirement end at server-observed session end (processlist absence on `kuru/<branch>`), bounded by the existing `QUERY_TIMEOUT` retirement deadline and tagged `PoolRetirement`. Place it per design D1, and use it before every delete, rename and exclusion probe. Verify tasks 1.1 turn green and the canary passes unmodified (verification 1.1, 1.2, 1.4).
- [ ] 2.2 Add the never-ending-session test with a test-supplied deadline. Verify the `PoolRetirement` failure, that no ref changed, that no `--force` was used and that the fence trips (verification 1.3).
- [ ] 2.3 Extend `candidate_branch_rename_reason` to label the exact in-use message at `Cleanup`. Verify only the diagnostic record changes (verification 1.5).

## 3. H2: never release a directory before its Dolt is reaped

- [ ] 3.1 Make every `MemoryStore::open` error after `Server::open*` await the owned server's bounded close (`close_installed_guard`/`close_migration_worker`) and attach a close failure as context. Sites: store.rs 1724, 1770, 1803, 1878-1890, 1910-1914, 1938-1939 on c986f4ff. Verify task 1.2 turns green.
- [ ] 3.2 Add `test_support::await_managed_quiescence` (`retire_idle_service`, then `Server::quiescence_at` within `SUPERVISOR_REAP_ALLOWANCE`). Use it in `kuru-runtime/src/dream.rs` and in every managed fixture that task 1.3 reported. Verify the invariant passes across the full kuru-memory and kuru-runtime suites with no test excluded.
- [ ] 3.3 Re-run the 0.2 baseline loop and suites with tracing. Record the after-counts (verification 2.3).

## 4. Windows cancelled activation

- [ ] 4.1 Give `StagedActivation` an explicit teardown for the unconsumed case: drop source and probe, run `staging.close_or_keep()`, surface a `StageCleanupFailure`, then release the lock. No new retry; the bound is the existing `CLEANUP_RETRY_LIMIT`. Verify with a forced cleanup failure (verification 3.2).
- [ ] 4.2 Verify the provision native tests on windows-latest native CI with unchanged assertions, and record the run and job ids (verification 3.1).
- [ ] 4.3 Re-examine item 1's CI log and record its disposition (verification 3.3).

## 5. Documentation and verification

- [ ] 5.1 Document the fixture teardown invariant and `await_managed_quiescence` in `docs/development.md`, and verify the docs checks pass.
- [ ] 5.2 Run the repository gates and the coverage gate, and record the results (verification 4.3, 4.4).
- [ ] 5.3 After push, collect the named macOS evidence set and the Ubuntu/Windows runs, and record every run and job id (verification 4.1, 4.2).
- [ ] 5.4 Update `tmp/roadmap` dx-followons §16 as superseded, and route items 13 and 14 plus the 10/11 deadline sub-items to the first-launch budget item (untracked notes, not part of the commit).
