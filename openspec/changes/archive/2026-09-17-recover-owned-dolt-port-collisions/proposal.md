## Why

An owned Dolt supervisor can lose its freshly reserved loopback port before spawn, then exit before readiness because an unrelated process occupies that selected port. The lifecycle lease remains owned, but the current single attempt fails without choosing a new private port.

## What Changes

Retry only an owned pre-publication Dolt startup that exits before readiness after full reap and drained trusted diagnostics prove the exact selected-port collision. Rebind a fresh loopback port under the existing startup deadline for at most three attempts; preserve all other startup, cancellation, cleanup, and endpoint-publication failures.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-memory/src/server.rs` and its real-Dolt lifecycle tests. No public configuration, process-kill, state-expiry, or deadline changes.

## Surfaces

- [ ] interactive
- [x] deploy
- [ ] integration
- [ ] agent-behavior
