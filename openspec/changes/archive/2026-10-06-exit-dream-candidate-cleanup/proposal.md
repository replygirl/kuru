# Proposal

## Why

Merged-main CI failed the real headless exit-dream cancellation fixture: the completed turn was retained, but shutdown returned a candidate-attachment cleanup error instead of typed cancellation. Parallel dream actors can still be awaiting private candidate reads when SIGINT cancels their callers; dropping a sent read loses its connection-owned candidate exchange without creating a mutation receipt.

## What Changes

Keep an already-dispatched nonmutating candidate exchange owned through its actual bounded response even if its caller is cancelled. Existing attachment serialization makes exact candidate cleanup wait for that read; preserve the no-reconnect guard and all mutation outcome fences, without replay or new protocol.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing cancellation, candidate ownership and completed-turn retry contracts are correct.

## Impact

Memory facade candidate-read lifetime and one real managed regression; existing headless exit-dream fixture and closing guard provide integration acceptance. Update owning memory documentation. No dependency, schema, wire, authentication or provider changes.

## Surfaces

- [x] interactive — SIGINT during the existing headless exit dream settles cleanup and preserves completed retry.
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
