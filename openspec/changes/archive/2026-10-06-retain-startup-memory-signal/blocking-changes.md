# Dependencies

## Blocked by

- [x] `legacy-import-reconciliation` — explicit owned import settlement *(archived 2026-10-05)*
- [x] `unix-update-recovery` — automatic retained installation recovery *(archived 2026-10-05)*

## Soft-blocked by

None.

## Coordination

The director accepted this narrow integration fix and normal merging of both clean archived histories. The worktree has no other active change. M1 is separate WIP, not a dependency: its future Backup/Verify/Restore callers will normally adopt the shared helper's third invocation-listener argument. Prior archives remain byte-for-byte unchanged. Update retains its separate existing listener; no general command cancellation or exit policy is added.
