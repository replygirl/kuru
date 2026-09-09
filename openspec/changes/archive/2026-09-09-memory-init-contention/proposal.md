## Why

Concurrent first opens can fail with SQLITE_BUSY while switching an identified memory database to WAL, even though a five-second busy timeout is configured. The pre-push regression failed within 50 milliseconds because SQLite can deliberately bypass its busy handler during lock promotion.

## What Changes

Give only the initialization journal-mode transition a bounded retry for SQLITE_BUSY, releasing each failed statement before another attempt. Preserve transactional schema initialization, validation before journal mutation, immediate reporting of other errors, and normal connection busy handling after initialization.

## Capabilities

### Modified Capabilities

None. The existing persistence and concurrent-handle contract remains correct; its implementation has a race.

## Impact

Changes packages/kuru-core/src/memory.rs and memory tests. No API, schema, dependency, tool or configuration change. Additional real SQLite and child-process fixtures verify contention handling and durable writes.

## Surfaces

- [ ] interactive
- [ ] deploy
- [x] integration — SQLite journal transition and lock handling
- [ ] agent-behavior
