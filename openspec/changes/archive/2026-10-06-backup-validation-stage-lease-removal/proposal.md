# Proposal

## Why

Native Windows backup validation keeps a second stage-directory handle alive
while checked removal must establish actual name absence. Final M1 CI reported
the retained validation stage with `removed directory name is still occupied`
in direct prepared-backup and historical-restore cases on x64 and ARM.

## What Changes

Use the existing consuming lifecycle-lease removal API after the same stage
identity checks and original-handle drop. That API consumes the checked root
directory while retaining the lifecycle lock through native removal. Remove
only the redundant second directory open and its duplicate identity check;
preserve initial/final revalidation and the seal's exact-identity check.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing settled-cleanup and checked-removal requirements are correct.

## Impact

Only `packages/kuru-memory/src/backup/native.rs` production code changes.
No platform removal policy, API, protocol, timeout, dependency, scanner,
fixture assertion or maintenance authority changes. Historical dirty restore
and the existing backup closing guard are scoped local acceptance; fresh native
Windows/full CI must establish the corrected public head. Managed backup/EOF/CLI
generic errors are not independently proven to share this cause.

## Surfaces

- [ ] interactive — no command or terminal interface changes
- [ ] deploy — no execution topology or workflow changes
- [ ] integration — existing internal checked filesystem API only
- [ ] agent-behavior — no prompt, routing or agent changes
