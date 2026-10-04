# Verification

## 1. Diagnosis [critical]

- [x] 1.1 @runtime (agent) run 37164260608 job 111324390127 log and `ci-coverage-diagnostics-ubuntu-latest-partition-8-attempt-1` -> observed: 271 tests passed; last executable `kuru_runtime-c677c8631f83e0f5` (35 tests) finished 00:29:39.06 with `tests::provider_free_undo_preserves_sessions_and_archives_added_identities` last; error at 00:29:45.22 "new kuru-13664-10634524661574408682_0.profraw"; runner ledger has counts only (kuru_runtime 496 -> 545) and no gaps between executables; no artifact names a profile, so the CI signature's binary is not recoverable from the run
- [x] 1.2 @equivalence (agent) partition 8's 35 `kuru_runtime` tests, instrumented locally (macOS, cargo-llvm-cov 0.9.1), runner-like group SIGKILL then 60 s watch; and its 91 `kuru_memory` tests with a 150 s watch -> observed: runtime 51 profiles, 2 signatures (test exe; every child = `target/debug/kuru-memory`), 0 late; memory 241 profiles, 0 late; the lifecycle trace shows 15 runtime tests drop a live store without close
- [x] 1.3 @regression (agent) `provider_free_undo…` alone (so it is the last test), same harness, ×3 -> observed: 3/3 a new `kuru-<pid>-<kuru-memory signature>_0.profraw` 0.25 s after the test process exited, its pid the Dolt supervisor whose trace names that test (spawn, stop_begin, child_exit Graceful after the test returned)

## 2. The test awaits its supervisor [critical]

- [x] 2.1 @regression (agent) `provider_free_undo…` with the new assertion and without the close, then with `drop(memory)`, then with `memory.close()` -> observed: without close FAILED "Dolt supervisor 3 … has not been reaped"; with drop FAILED "… was dropped without a close, so its reaper thread, not the test, awaited its exit"; with close passed ×3 (1.71 s, 2.70 s, 2.03 s)
- [x] 2.2 @runtime (agent) fixed `provider_free_undo…` alone, instrumented, runner-like harness ×3; unfixed sibling dropper `dreaming_rejects_last_role_removal…` ×3 -> observed: fixed 0 late profiles in 3/3; dropper late in 2/3 (0.25 s), each late pid matching that test's `dolt-supervisor` spawn row
- [x] 2.3 @unit (agent) `test_support::engine_ledger::tests` and `test_support::spawn_ledger::tests` ×3 -> observed: 5 passed each run, including `only_a_close_awaits_a_marked_tests_supervisors`

## 3. A late profile names its writer [critical]

- [x] 3.1 @unit (agent) `cargo test -p kuru-delivery --lib --all-features -- coverage::` ×3 and `mise run //packages/kuru-delivery:test -- coverage::` -> observed: 108 passed each, including `a_late_profile_names_the_test_that_started_its_process` (CI shape: "pid 12 is the dolt-supervisor debug/kuru-memory started by test tests::b … signature 222 is debug/kuru-memory"), the spawns module tests and the dispatch test's listing rows
- [x] 3.2 @integration (agent) partition 8's runtime set, instrumented, with `KURU_COVERAGE_SPAWN_LEDGER` ×3 -> observed: 49 profiles, 48 rows, every child profile's pid has a row; owners and the supervisors they spawn carry the originating accounting test's name
- [x] 3.3 @integration (agent) `terminal` PTY tests `real_pty_marker_lines…` and `real_pty_accepts_chat_navigation…` with the ledger ×3 -> observed: 2 passed each; 14 rows: terminal children, their owners and supervisors labelled with the test; owners of non-terminal `kuru` children still read `main` (their spawner is not recorded)
- [~] 3.4 @runtime (agent) a native CI coverage shard with this change -> defer: no CI rerun is allowed for this task; the PR's own CI runs every partition

## 4. Static checks

- [x] 4.1 @unit (agent) `mise run format:check`, `lint`, `lint:windows`, `typecheck`, `lint:tooling`, `docs:check`, `cospec -- validate --all --strict` -> observed: each exit 0
