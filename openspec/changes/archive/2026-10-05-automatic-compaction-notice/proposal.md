# Proposal

## Why

Automatic per-actor compaction currently discards its accepted checkpoint outcome, so the user can lose the context-maintenance notice even though a new summary and cursor are durable. Cancellation after acceptance and a lost checkpoint reply also need exact confirmation before any notice can claim success, while private summaries and original histories must remain private and preserved.

## What Changes

- Emit a metadata-only notice for each confirmed automatic or manual per-actor compaction, identifying the actor, covered source range, summary identity, selected live or candidate view and retained originals.
- Retain only compaction attempt/notice metadata in the current Harness, publish acknowledged acceptance before the subsequent cancellation check, and recover delivery through existing typed events and operation-settlement drains without duplicate notices.
- Add one memory-owned read-only exact summary-identity metadata confirmation so an aborted or uncertain checkpoint can be confirmed after existing exact write-fence recovery, including when a later summary has superseded its cursor.
- Show notices through the existing TUI presentation and headless stderr paths without exposing private bodies, contaminating stdout, replaying inference, changing compaction policy or changing durable journals, receipts or TurnOutput.
- Route headless Run Ctrl-C to its existing cancellation token and await the same controlled operation before existing shutdown/lease cleanup and confirmed stderr draining; preserve completed answers and current result classification.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `actor-context-compaction`: Reliable metadata notice delivery for confirmed automatic/manual checkpoints and cancellation/recovery parity.
- `project-memory-service`: Read-only exact context-summary identity confirmation on the selected view without returning its private summary body.

## Impact

The scoped implementation touches runtime actor/engine/event handling and accounting fixtures, the memory facade/store/RPC confirmation projection and owning fixtures, TUI settlement and CLI stderr presentation with observable fixtures, and compaction documentation. Storage split B and the composer slice must first be integrated normally into this branch so their existing ownership and publication semantics remain intact. No storage migration, durable notice outbox, generic notification framework, provider routing, retention policy or new dependency version is introduced.

## Surfaces

- [x] interactive — TUI and headless user-visible compaction notices
- [ ] deploy — no deployment or runtime topology change
- [x] integration — one read-only operation on the existing private memory service contract
- [ ] agent-behavior — no prompt, policy, routing or model-loop change
