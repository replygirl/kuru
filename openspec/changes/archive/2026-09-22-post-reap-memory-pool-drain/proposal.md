## Why

An Intel macOS native memory run reached real SQL-session teardown, then `MemoryStore::close` failed after the fixed eight-second pool-close deadline with one idle connection still closing. `Server::close` did reap its owned Dolt process, but returned the earlier pool timeout without checking whether the now-stopped engine let the closed pool finish draining. This reports shutdown failure even when the required lifecycle cleanup can complete immediately after owner reap.

## What Changes

Keep the initial bounded graceful pool drain. If it times out, reap the exact retained owner, then give the already-closed pools a separately bounded final drain. Return success only after owner reap and pool cleanup both complete; retain a diagnostic error when either cannot be proven. Add a real-Dolt fixture that holds a pool connection through the first drain and releases it only after verified owner reap.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The living explicit shared-store shutdown requirement already requires awaited owner and pool cleanup.

## Impact

`packages/kuru-memory/src/server.rs` and a native memory recovery fixture. No public API, storage schema, dependency, or runtime topology change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface
- [ ] deploy — deploy/runtime/CI-execution topology
- [ ] integration — a third-party/external contract
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
