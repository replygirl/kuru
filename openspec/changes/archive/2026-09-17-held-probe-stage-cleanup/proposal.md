## Why

The held cold-probe fixture intentionally retains a Windows no-DELETE handle on an isolated probe image. Candidate activation succeeds, but the new explicit production stage close truthfully reports its unable-to-remove stage instead of silently discarding that cleanup error.

## What Changes

- Update `packages/kuru-memory/src/provision/native_tests.rs` to expect the post-publication cleanup error while preserving destination identity, payload, probe-handle, cache-lease, and manual retained-stage cleanup assertions.

## Impact

Test-only native Windows expectation correction; no production behavior, retry, deadline, seam, or CI topology change.
