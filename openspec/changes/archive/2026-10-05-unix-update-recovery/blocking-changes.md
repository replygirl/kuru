# Dependencies

## Blocked by

- [x] `update-install-ownership` — checked Unix manager refusal and retained installation preflight *(archived 2026-10-05)*
- [x] `concurrent-session-admission` — current checked service/driver semantics for running-image acceptance *(archived 2026-10-05)*

## Soft-blocked by

None.

## Coordination

The director assigned normal integration of clean 7a84ab31 into this branch before the implementation gate. Both providers are actually archived and locally committed; this dependency record does not claim hosted delivery. The only active change in this worktree is this change. Archived usage/shell-support, native Windows update and project-service capabilities are retained, and the established platform primitives remain the mechanism.

M1 backup and M2 legacy reconciliation are independent changes in separate worktrees. This change does not touch their SQL inventories, activation/import or backup contracts. Coordinate the narrow self-image selection and supervisor launch with the memory owner. Update notices remain a subsequent D2 change. The scope comes from the canonical D2 outcome, author P25 design and Current handoff Q-E plus current engineering direction; it does not elevate review suggestions to original human decisions.
