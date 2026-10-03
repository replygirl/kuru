# Verification

Observed 2026-10-03, macOS arm64, worktree on origin/main 65d64209. Red on the
test commit 479b2a86 (no fix); green on the fix commit b93a2b63.

## 1. Maintenance outlasts a close held past the startup timeout [critical]

- [x] 1.1 @regression (agent) `mise run //packages/kuru-memory:test -- maintenance_behind_a_close_held` on 479b2a86 (owner held at `ClosePoint::AfterReap`; paused time, the acquisition's sleeps and `timeout_at` are tokio timers and every request is a record read with no socket) -> observed exit 101, `0 passed; 1 failed` (1.51 s wall): `maintenance ended within 31s while the owner's close was held: Err("memory service owner is still active; maintenance cannot proceed; maintenance waiting for the owner lock for 0ms (this lock's wait 30098ms); requests without a live endpoint=299; ...")`
- [x] 1.2 @regression (agent) `mise run //packages/kuru-memory:test -- --lib -- maintenance_behind_a_close_held purge_refuses maintenance_names_a_live` on b93a2b63, three times -> observed each exit 0, `4 passed; 0 failed` (35.26 s, 34.73 s, 34.80 s; the live-owner test runs the 32 s budget on the real clock)

## 2. Existing deadline behaviour

- [x] 2.1 @integration (agent) `mise run //packages/kuru-memory:test -- --lib -- maintenance_ purge_refuses an_elapsed_retirement a_retirement_behind_a_close` with the fix -> observed exit 0, `13 passed; 0 failed` (45.97 s): the error texts are unchanged; the purge refusal test (paused time) asserts its wait reached `close_budget()` with `startup_timeout_secs = 1`; the retirement fixture tests pass unchanged
- [x] 2.2 @integration (agent) full `mise run //packages/kuru-memory:test` on b93a2b63 -> observed exit 0 (842 s): lib `708 passed; 0 failed; 6 ignored` (811.93 s); bundle_build 10, memory 5, server_lifecycle 12, supervisor_snapshot 1 passed

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> observed each exit 0 (`NODE_OPTIONS` unset); validate 0 errors, 0 warnings
- [~] 3.2 @runtime (agent) native PR CI partitions -> defer: CI runs after the push; CI reruns are not used as evidence here
