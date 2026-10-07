# Dependencies

## Blocked by

- [x] `concurrent-session-admission` — checked driver/native maintenance ownership and expected-existing recovery *(archived 2026-10-05)*
- [x] `reconcile-dream-candidates` — complete retained candidate provenance and exact reconciliation outcomes *(archived 2026-10-05)*
- [x] `ordered-dolt-migrations` — released version registry, isolated migration construction and checked publication *(archived 2026-09-13)*

## Soft-blocked by

None.

## Coordination

The parent authorized this dependency sequence and scope from clean N4 commit `421a15b944b735ed1a2048d93a85c9b3c8075cbc`. No other active change exists in this worktree. D2 and O3 run in separate worktrees; shared CLI/facade/service hunks integrate from stable commits, without simultaneous writes or a new product gate. Hosted acceptance and remote publication remain separate from this local implementation gate.
