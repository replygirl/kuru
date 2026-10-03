# Tasks

## 1. Diagnose at cause

- [x] 1.1 Read run 37147327774's partition-3 diagnostics, logs and runner ledger, and verify which reports ran when and that partitions 1, 2 and 4 of the same commit reproduced their summaries
- [x] 1.2 Reproduce the shape locally with an instrumented stand-in test: export before and after the late stand-in profile, and verify the port reproduces each export's own summary while the cross-set comparison fails as in CI

## 2. Stand-in owner never outlives its test

- [x] 2.1 `stand_in_owner` removes its release file before exiting; the `progress_wait` fixture writes only its first release and waits in real time for consumption once the stand-in was spawned; verify the `progress_wait` tests pass
- [x] 2.2 Add `a_stand_in_has_consumed_its_release_when_its_start_returns`, and verify it fails with the fix reverted and passes with it

## 3. Partition holds one profile set

- [x] 3.1 Record the raw profile set after the tests and require it unchanged after the last report (before the self-check) and after the receipt; verify the coverage module tests pass
- [x] 3.2 Add the fake-host regression test for a profile written during each report and the receipt, and verify it fails with the guard removed (the CI port message) and passes with it

## 4. Documentation and checks

- [x] 4.1 Update docs/development.md and verify `mise run docs:check`
- [x] 4.2 Run the checks in verification.md and record the evidence
