# Dependencies

## Blocked by

- [x] `split-topology-persistence` — disjoint membership/report/session storage and coherent candidate topology reads *(archived 2026-10-05)*
- [x] `session-scoped-memory-provenance` — session-filtered source snapshots, conditional summaries and project dream ownership *(archived 2026-09-22)*
- [x] `await-candidate-session-retirement` — checked candidate transition retirement with owned server-session completion *(archived 2026-09-24)*

## Soft-blocked by

None.

## Coordination

Root assigned N3 on accepted clean local B commit 101add0255c1122e94fe11cb594429576049020b; B PR231 hosted checks and merge are still pending, so no claim of shipped native acceptance is made. Current three-PR delivery queue must move before another PR is published. Normal main restacking precedes delivery.

No other active change exists in this worktree. C1's compaction notice owns actor/event wiring and one read-only exact summary-ID metadata query; it does not provide a business prerequisite for candidate reconciliation. Its accepted archived commit `04215130` and B's proven checked-rebind correction `e22090bc` are normally integrated in `3feb452`; checked remote main `a1bd4836` is normally integrated in `ba228e64`. Shared engine/facade/RPC seams are reviewed on the combined source and use one minor 1.11 pin. N4 admission follows this work and remains disabled here. Delivery queue state is rechecked before publication; this record does not claim a hosted merge from a local dependency.
