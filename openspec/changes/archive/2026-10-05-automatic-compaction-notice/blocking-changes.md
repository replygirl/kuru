# Dependencies

## Blocked by

- [x] `split-topology-persistence` — integrate the settled runtime publication, candidate and storage read ownership before changing those shared paths *(archived 2026-10-05)*
- [x] `phase2-composer-parity` — integrate the stable TUI editing/render state before changing its shared settlement presentation *(archived 2026-10-05)*

## Soft-blocked by

None.

## Coordination

The maintainer's active Phase 2 assignment explicitly requires normal integration of B's final commit and the composer slice's stable UI commit before implementation. Both changes are being delivered in independent worktrees; this fresh main-based worktree has no other active provider change. Existing actor compaction, atomic checkpoint provenance and typed events are already archived on its baseline. No additional product approval or release cycle is introduced by these integration dependencies.
