## Why

Linux CI run 34403343728 failed the independent-handle test with SQLITE_BUSY during two uncoordinated bursts of forty durable writes. The test assumes the waiting writer receives a turn before the five-second busy budget expires, but SQLite does not promise fairness between successive transactions.

## What Changes

- Preserve eighty writes across two independent handles, contending in pairs so one unbounded burst cannot repeatedly reacquire the writer lock ahead of its peer.
- Verify the result after reopening the database and keep independent-process coverage.
- Add explicit held-lock tests for bounded busy failure, unchanged data and successful operations after lock release.
- Retain production transaction behavior, timeout, durability and error handling.
- Record the user's selected Dolt transition as a separate post-release PR in canonical agent guidance.

## Capabilities

### Modified Capabilities

None. The storage contract already allows bounded contention failure.

## Impact

Memory integration tests, the cfg(test) unit tests inside packages/kuru-core/src/memory.rs, and AGENTS.md's planned storage direction. No schema, dependency, workflow, runtime or timeout changes.

## Surfaces

- [ ] interactive
- [ ] deploy
- [x] integration — real SQLite lock and timeout contract
- [ ] agent-behavior
