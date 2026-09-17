## Why

A Windows native fixture observed a pool-close deadline after it had completed its
owned query and session teardown. The current fixture preserves the close error
without the bounded pool state needed to diagnose that failure.

## What Changes

- Capture the retained test pool's size, idle count and closed state before and
after the fixture's existing `MemoryStore::close` call, with elapsed close time,
only when that original call fails.
- Preserve the query, teardown and status assertions; add no production behavior,
retry, deadline or public API.

## Impact

Touches one real-Dolt recovery fixture and its test-only evidence. Native Windows
verification is required to observe the diagnostic in the failing environment.
