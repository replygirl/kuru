## Why

`MemoryStore::close` currently takes `&self`, although it shuts down the shared
Dolt server and closes every clone's pool. That makes ordinary view release easy
to confuse with explicit shared-server shutdown, and read methods can instead
reach SQL after a close while writes report the clear `memory store is closed`
error. The public reconciliation result and revision ordering also conceal
durable outcome and graph-order information that callers need for honest,
deterministic inspection.

## What Changes

- Make explicit `MemoryStore::close` consume its view and document that it
  shuts down the shared server and all views; ordinary `Drop` releases one view.
- Reject reads through a closed shared pool with the same closed-store error as
  writes, and expose reconciliation's existing `None`/`Some(true)`/
  `Some(false)` outcome.
- Return revisions in pinned-Dolt graph order with a stable hash tie-break and
  prove tied wall-clock timestamps do not replace ancestry order.

## Capabilities

### New Capabilities

- `memory-store-lifecycle`: explicit shared-store shutdown, closed-view
  failures, reconciliation outcomes, and deterministic revision inspection.

### Modified Capabilities

<!-- None. -->

## Impact

- `packages/kuru-memory/src/store.rs` and its focused real-Dolt fixtures.
- Memory lifecycle and history documentation.
- No schema, migration, ownership-token, journaling, retention, or erasure
  behavior changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
