## Why

The file checkpoint store treats dropping `CheckpointLease` as the end of its
exclusive advisory-lock ownership. On Unix, `flock` belongs to the open file
description, so a duplicate descriptor can retain that lock after the lease's
own `File` closes; a later undo then reports `file checkpoint store is busy`
despite the first logical lease having ended. A macOS CI test observed that
error, but its exact concurrent-spawn timing is unproven. A deterministic
same-description duplicate will establish and correct the lease-lifetime defect
without relying on that historical timing.

## What Changes

- Add a focused regression that holds a real checkpoint lease, duplicates its
  lock handle, proves an independently opened lease is excluded while the
  original is active, then proves it can acquire immediately after the original
  lease ends even while the duplicate remains open. Record the expected red
  result before changing production code.
- Make `CheckpointLease` explicitly release its advisory lock when its owned
  lifetime ends. Keep the existing checked lock object, mutual exclusion,
  receipt verification and no-retry semantics.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The existing checked file-checkpoint ownership contract is correct; this
fix aligns its Unix lock lifetime with that contract.

## Impact

Only `packages/kuru-connectors/src/file_edits.rs` and its directly owned unit
fixture change. There is no schema, wire, configuration, provider, platform API,
release workflow or dependency change.

## Surfaces

- [ ] interactive — no command surface or user flow changes.
- [ ] deploy — no deployment or runtime topology changes.
- [ ] integration — no external protocol or schema changes.
- [ ] agent-behavior — no prompt, model-routing or tool-output shape changes.
