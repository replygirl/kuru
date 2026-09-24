## 1. Physical session provenance and legacy preservation [critical]

- [x] 1.1 @integration (agent) upgrade a released schema-4 real-Dolt store containing typed and legacy rows, append two attributed sessions, and inspect SQL plus typed APIs -> original rows retain exact payloads/sequences with null session, new rows retain their explicit session, and the global sequences are unique and ordered
- [x] 1.2 @integration (agent) full-memory export attributed rows, unattributed rows, summaries and cursors -> every optional identity and provenance coordinate is preserved while legacy rows remain inspectable and excluded from session continuity

Observed 2026-09-22: the exact released-v4 migration fixture passed 1/1 after exposing and correcting information-schema label case and fresh usage-branch schema inheritance. The exact local contract/export fixture passed 1/1 and retained null legacy rows, session rows, one summary and one cursor.

## 2. Revision-bound source snapshots [critical]

- [x] 2.1 @integration (agent) read interleaved sentinel rows for two sessions through a real live store -> each bounded page contains only its requested session in sequence order and reports the exact pinned view/revision and derived inclusive boundary
- [x] 2.2 @integration (agent) create a candidate with private session rows and compare its snapshot with live -> candidate-only content stays on the candidate, and both results name their actual distinct view/revision
- [x] 2.3 @regression (agent) request zero/oversized pages, invalid identities and namespace-only fallback -> every request fails within the documented bounds without returning another session or null-attributed legacy row
- [x] 2.4 @regression (agent) request individually valid source rows that cross the aggregate response budget -> the store streams at most 1,024 rows and 32 MiB of serialized rows, derives `through` from the last retained row and never constructs an over-frame reply

Observed 2026-09-22: the exact local live/candidate source fixture passed 1/1. The extended fixture passed 1/1 in 2.77s after one transaction seeded 1,025 valid rows; the page retained exactly 1,024 and derived `through` from its retained last row. The shared row/byte budget regression passed 1/1 and proved rejected next-row/next-byte accounting is non-consuming at the exact 32 MiB boundary.

## 3. Atomic conditional summary and cursor [critical]

- [x] 3.1 @integration (agent) publish `context_summary.v1` for an unchanged real-Dolt snapshot -> summary provenance and the actor/session/source cursor appear in one committed revision and typed reads observe both
- [x] 3.2 @integration (agent) race a sibling append and a competing cursor advance before publication -> revision-stale and cursor-stale calls return the typed no-effect error with no summary or partial cursor movement
- [x] 3.3 @integration (agent) interrupt an accepted managed checkpoint reply and query its exact receipt -> committed work reconciles once from the original complete fingerprint, while an unproved outcome fences later mutation and is never replayed after reconnect
- [x] 3.4 @regression (agent) submit a current-revision checkpoint whose caller-selected range exceeds the source row or byte page ceiling -> typed stale rejection occurs before either summary or cursor insert

Observed 2026-09-22: the exact local conditional checkpoint fixture passed 1/1, including revision-stale and cursor-stale no-effect checks. Its extended 1,025-row case passed in the same fixture and returned typed stale for the current-revision over-bound range with neither summary nor cursor inserted. The exact managed lost-reply fixture passed 1/1, including receipt reconciliation, typed stale reconstruction and later continuation without a fence.

## 4. Managed facade and admission boundary

- [x] 4.1 @equivalence (agent) exercise append, snapshot and checkpoint through local and authenticated remote live/candidate views -> DTO validation, results and typed stale errors match without exposing SQL or backend handles
- [~] 4.2 @regression (agent) run the existing writer/driver lease fixtures with the new storage API -> defer: the focused managed checkpoint and dream-lease fixtures proved remote/candidate equivalence and ordinary append availability, while Delivery's final normal hook owns the unchanged full admission regression on the integrated head

Observed 2026-09-22: the local provenance fixture and authenticated managed lost-reply fixture passed 1/1 each, including main/candidate append, snapshot, history, checkpoint, typed stale and current-summary reads. Final `kuru-memory` and `kuru-runtime` all-target/all-feature typechecks passed in 6.98s and 7.97s. The exact-head bundled supervisor prefetch passed in 12.51s. No full suite, coverage or hosted native matrix was run for this P29 source head.

## 5. Session-scoped runtime continuity

- [x] 5.1 @integration (agent) seed one actor namespace with current-session, other-session and null-attributed raw rows plus a policy-selected note, then run that actor -> provider context contains current-session raw history and the selected note but excludes the other-session and legacy rows
- [x] 5.2 @equivalence (agent) query bounded session history through local/remote live/candidate views -> total counts, newest retained suffix, byte bounds and pinned-view isolation match without exposing namespace-only history
- [x] 5.3 @equivalence (agent) checkpoint two rolling summaries and query the exact-session plus policy-authorized shared projection through local/remote live/candidate views -> only each cursor-selected current summary identity appears, the response names its pinned view/revision and export retains the superseded record

Observed 2026-09-22: the session-history/context runtime fixture passed 1/1 and admitted only current-session raw rows plus the selected note. The local source/summary fixture passed 1/1 with newest-suffix/count-only reads, exact session/source and broader current-summary selectors, pinned candidate isolation and retained historical export rows. The managed lost-reply fixture passed 1/1 and read the exact current summary through the remote facade after receipt reconciliation.

## 6. Coherent topology and serialized dreams

- [x] 6.1 @regression (agent) commit a changed topology, admit a new turn and retry an already completed turn after corrupting the later stored topology -> the new turn captures the valid changed graph once, while the completed retry returns its exact stored output without provider work or topology mutation
- [x] 6.2 @integration (agent) hold one managed dream lease while a second waits and an ordinary memory append settles, then cancel a waiting acquisition -> dreams serialize, chat remains available and cancellation/disconnect leaves no lease behind
- [x] 6.3 @integration (agent) advance live memory after a dream captures its base and before exact promotion -> the typed conflict retains candidate/report and current live history, publishes no candidate topology and requires the existing checked resolution without automatic inference replay
- [~] 6.4 @e2e (agent) launch two normal CLI/TUI processes and enforce one driver per live session -> defer: P30 owns user-visible concurrent admission after P09/P11/P29 integration

Observed 2026-09-22: the valid next-turn topology refresh and completed-turn retry ordering fixtures passed 1/1 each. The managed dream-lease fixture passed 1/1 while an ordinary append settled and a cancelled waiter released cleanly. The stale dream candidate fixture passed 1/1 and retained its report/open ref through typed conflict until explicit abandon. The existing single conversation-driver admission remains in force; P30 owns concurrent user-visible admission.
