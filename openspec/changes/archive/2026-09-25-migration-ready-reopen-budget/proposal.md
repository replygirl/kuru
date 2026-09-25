## Why

The real-Dolt retained-ready migration recovery test can time out after 10 seconds while reopening a writable store, even though the configured startup and migration observation budget is longer. A native Intel macOS run failed with an unlabelled deadline error, leaving the failed phase ambiguous.

## What Changes

- Use the existing migration observation budget for that test's final reopen, while retaining its short fault-control waits and all recovery assertions.
- Label the final reopen's outer deadline separately from an error returned by the open itself.

## Impact

Only `packages/kuru-memory/src/store/recovery_tests.rs` changes. The test may wait up to the existing configured migration observation budget on failure; product startup and migration deadlines do not change.
