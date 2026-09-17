## Why

The real-Dolt cyclic-peer regression test has a two-second outer test safety bound that can expire under instrumented Windows execution even though the bounded two-round behavior is correct.

## What Changes

- Extend only the test's outer safety timeout and report the elapsed bound and observed request count if it expires.
- Preserve the existing two-round, `PeerRounds`, request-count, and budget assertions.

## Impact

Touches `packages/kuru-runtime/src/tests.rs`; it affects only an exceptional test-failure wait.
