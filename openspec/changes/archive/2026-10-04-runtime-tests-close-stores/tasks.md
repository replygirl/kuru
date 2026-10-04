# Tasks

## 1. Measure

- [x] 1.1 Trace the whole `kuru_runtime` lib on main and verify how many tests record `owner_dropped_live`
- [x] 1.2 Read run 37186425537 partition 6's log and diagnostics artifact and verify what identifies the late profile's writer

## 2. Teardown scope

- [x] 2.1 Add `kuru_memory::test_support::closing` and test-support registration of local stores, and verify unit tests for close-on-success, close-on-panic (panic resumed), close-on-error-return and the unawaited assertion
- [x] 2.2 Wrap every `#[tokio::test]` body in `kuru-runtime` in `closing`, remove the redundant trailing `close_stores` calls, and verify the full package test passes
- [x] 2.3 Add the source-scan guard and verify that it fails, naming the test, for an unwrapped test

## 3. Regression evidence

- [x] 3.1 Inject a failing assertion before teardown into `dreaming_rejects_last_role_removal…`, run it instrumented like the runner on main and on the fix, and verify late profiles are above 0 before and 0 after
- [x] 3.2 Re-trace the whole lib and verify 0 `owner_dropped_live`

## 4. Checks

- [x] 4.1 Run `//packages/kuru-runtime:test`, the touched tests ×3, `format:check`, `lint`, `lint:windows`, `typecheck` and `cospec validate --all --strict`, and record each exit code in verification.md
