# Proposal

## Why

Each public transcript page currently repeats the selected session's entire predecessor metadata walk, even after its bounded response is full. A sequential 1,025-turn scan at 32 records per page makes 33 full walks; response bounds and the short writer guard do not bound that repeated work.

## What Changes

Reuse a fully validated exact-cut chain proof within one pinned view/authenticated attachment. Retain only totals and the issued entry/next positions of two returned pages, at most four positions and 2 KiB; ordinary clones share the slot and independent attachments/candidate views start empty. Every request still checks the actual selected HEAD and catalog before opening the immutable pool. Cache misses, forgotten positions and forged cursors use the existing full proof. Publish a new proof only after successful transaction and final bounded response validation.

Initial projection remains linear in chain length. Sequential older-page navigation/search becomes one initial chain walk plus bounded page work. Existing newer-page reconstruction may still repeat prefix pages; this change does not redesign it.

## Capabilities

### Modified Capabilities

- `interactive-transcript-navigation`: reuse exact immutable chain proof for bounded issued continuations without removing reachability, privacy or stale-source checks.

## Impact

Memory store public paging, attachment-local view cloning, existing real-memory paging fixtures and interface documentation. No schema, indexes, RPC shape, UI, provider, authentication or dependency changes.

## Surfaces

- [x] interactive — storage latency of existing scrollback/search
- [ ] deploy — no execution topology change
- [ ] integration — no external contract change
- [ ] agent-behavior — no prompt/output change
