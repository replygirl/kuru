# Proposal

## Why

The retained-supervisor fixture returned an unqualified Windows access-denied error, leaving the failing operation unknown. Its removal observer must confirm absence rather than confuse an uncertain deletion-pending observation with either success or a proven cleanup failure.

## What Changes

- Add operation context to retained-directory handoff, pool acquisition and removal observation in `packages/kuru-memory/tests/server_lifecycle.rs`.
- Add a causal native Windows held-handle case proving that owned removal remains pending until handle release and confirmed absence; retain uncertain observation errors within the existing deadline.

## Impact

Test file and typed change artifacts only. Preserve production ownership, original lifecycle assertions, the existing deadline and polling interval; no deletion retries, dependencies or platform interop additions. The historical failing operation remains unproved. Host runs the original retained lifecycle case and closing guard; native Windows executes the causal case in CI.
