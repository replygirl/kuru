# Verification

## 1. A legacy snapshot that keeps progressing completes [critical]

- [x] 1.1 @regression (agent) `run_backup` with a step closure returning `More` 100 times then `Done` and a mock clock advancing 10 s per reading (20 s between progress steps, about 2,000 s in total), against the old fixed-deadline loop and the fix -> Observed 2026-10-03, macOS arm64. Red with `run_backup`'s body reverted to the old semantics (deadline fixed at entry, old message): `snapshot_that_keeps_progressing_completes_past_the_stall_bound` FAILED with `called Result::unwrap() on an Err value: legacy memory snapshot deadline exceeded; close older writers and retry`. Green with the fix: passes, 101 steps observed.

## 2. A stalled legacy snapshot fails naming the stall [critical]

- [x] 2.1 @regression (agent) `run_backup` with a step closure returning `Busy` and a mock clock advancing 11 s per reading, asserting the exact new message, against the old loop and the fix -> Observed 2026-10-03, macOS arm64. Red: `snapshot_stalled_on_a_busy_source_fails_naming_the_stall` FAILED, left `"legacy memory snapshot deadline exceeded; close older writers and retry"`, right `"legacy memory snapshot made no progress for 30 s while the source was busy or locked; retry when it is free"`. Green: passes. The real 10 ms pause is not injected; the mock clock passes the bound after two `Busy` steps (the test asserts at most three), so it sleeps at most about 30 ms.
- [x] 2.2 @unit (agent) a step error propagates as the underlying `rusqlite::Error` -> Observed 2026-10-03: `snapshot_step_error_propagates` passes (red run included, since error propagation is unchanged).

## 3. Existing legacy import behavior is preserved

- [x] 3.1 @integration (agent) `mise run //packages/kuru-memory:test -- migration::` (the real legacy SQLite import tests live in `migration::tests`) -> Observed 2026-10-03, macOS arm64: exit 0; 11 passed, 0 failed (the 8 existing migration and WAL import tests plus the 3 new ones), 696 filtered out. The full memory package suite was not run (narrowed to the touched module).
- [x] 3.2 @integration (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> Observed 2026-10-03, macOS arm64: each exits 0 (lint 85 s, root lint:windows 91 s, typecheck 96 s, docs:check 6 s, validate "0 errors, 0 warnings"). format:check first reported one rustfmt line wrap in the new test; after `format:fix` it exits 0. format:check, lint and docs:check ran with `NODE_OPTIONS` unset because the shell's preload module breaks the docs toolchain step.
