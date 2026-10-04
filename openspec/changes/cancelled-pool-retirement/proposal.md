## Why

Main's native macOS runtime test observed an eight-second candidate pool retirement failure after canceling a dream. The artifact does not identify the exact SQL session, but the pinned SQLx return-to-pool path can retain a connection permit indefinitely while waiting for a canceled query's response; a controlled real-query test must establish this mechanism before changing it.

## What Changes

- Add a real-Dolt, protocol-opaque relay control that holds a completed query's response and distinguishes an already-started return task from the pre-close race.
- Make an abnormally dropped pooled session use SQLx's existing bounded close-on-drop path while retaining its connection permit through close.
- Preserve successful inline release, configured pool capacity, existing pool-close deadlines, exact server-session retirement and uncertain-write reconciliation.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `memory-store-lifecycle`: specify bounded retirement of a canceled pooled operation without waiting for its pending query response.

## Impact

The implementation is limited to `packages/kuru-memory/src/pool.rs` and a private cancellation test module. No dependencies, runtime behavior outside abnormal pooled-session disposal, configuration, database schema, workflow or allowance changes are planned. The historical CI cause remains unproven; local causal evidence and fresh native CI must be recorded separately.

## Surfaces

- [ ] interactive
- [ ] deploy
- [x] integration
- [ ] agent-behavior
