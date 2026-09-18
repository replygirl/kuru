## 1. The previously racy assertion is now deterministic [critical]

- [x] 1.1 @regression (agent) `KURU_DOLT_BUNDLE_DIR=/private/tmp/kuru-phase1-bundles RUST_TEST_THREADS=2 KURU_TEST_SUPERVISOR_PREPARED=1 cargo test -p kuru-memory --lib --all-features -- store::recovery_tests::production_upgrade_reconciles_lost_commit_reply_after_routed_session_ends store::recovery_tests::production_upgrade_reconciles_lost_branch_reply_after_exact_ref_creation store::recovery_tests::production_upgrade_reconciles_lost_fast_forward_reply_after_target_publication store::recovery_tests::absent_fast_forward_keeps_the_same_ready_attempt_for_next_open --exact --test-threads=2`, run three times in a row on macOS with the real bundled Dolt -> `test result: ok. 4 passed; 0 failed` on all three runs (5.96s, 5.29s, 5.31s). Before the fix, `absent_fast_forward_keeps_the_same_ready_attempt_for_next_open` was observed to fail intermittently on Windows CI (run 35382536799, job 105721841828) at the bare `assert!(proxy.session_ended.load(Ordering::Acquire))`; the fixture race that caused that is the same one bounded here, and the same assertion path is exercised by all four tests run above.
- [x] 1.2 @unit (agent) `cargo check -p kuru-memory --lib --tests --all-features` -> clean, no warnings introduced by `await_flag` or its four call sites (one pre-existing unrelated `dead_code` warning on `fixture_commit_malformed_state` predates this change).

## 2. No weakened assertion, no widened timeout, no product change

- [x] 2.1 @unit (agent) code review of the diff -> all four `session_ended` sites still assert the same fact (the proxy observed the routed session end) before proceeding; none were removed, relaxed, or replaced with an unconditional pass. `await_flag` polls the same `TEST_DEADLINE` (`Duration::from_secs(10)`) already used throughout this file for every other bounded wait in these same tests (`durable_observation`, `await_session_end`, `control.reached()`) — no new or widened timeout was introduced. No `#[ignore]` was added. No file outside `packages/kuru-memory/src/store/recovery_tests.rs` was touched; `server.rs` and all other product code are unchanged.
- [x] 2.2 @unit (agent) confirmed the `discarded` assertions in this file -> the two sites adjacent to the fixed `session_ended` sites, plus the other five `discarded` sites, are untouched; they retain a true happens-before edge via `compare_exchange` before the socket shutdown, unlike `session_ended`.

## 3. The repository's own gates

- [x] 3.1 @unit (agent) `mise run format:check` -> clean.
- [x] 3.2 @unit (agent) `mise run //packages/kuru-memory:lint` -> clean, `-D warnings`, all targets and features.
- [x] 3.3 @unit (agent) `mise run cospec:validate` -> passed.
- [~] 3.4 @runtime (agent) `mise run coverage` / `mise run check` -> defer: deliberately not run per this task's operating instructions; the workspace 90% gate is enforced by CI on the branch.
