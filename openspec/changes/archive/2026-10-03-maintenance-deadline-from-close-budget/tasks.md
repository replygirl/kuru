# Tasks

## 1. Regression test

- [x] 1.1 Add `service::tests::maintenance_behind_a_close_held_past_the_startup_timeout_acquires_within_the_close_budget` (owner held at `ClosePoint::AfterReap`, paused time carries the acquisition past `startup_timeout_secs + 1 s`, then the close is released) and verify it fails before the fix at the 30 s deadline

## 2. Fix

- [x] 2.1 Derive the maintenance permit deadline from the longer of `server::close_budget()` and `startup_timeout_secs` for the start-lock and owner-lock phases, keeping the error texts, and verify the regression test passes
- [x] 2.2 Adjust the purge refusal and live-owner-closing-connections tests to the new deadline (paused time where no socket is live; real clock where it is) and verify they pass

## 3. Docs and checks

- [x] 3.1 Update `docs/development.md` and `docs/memory.md` and the two `test_support.rs` comments and verify `mise run docs:check`
- [x] 3.2 Run the kuru-memory tests and the static checks and record them in verification.md
