## 1. Schema and physical provenance

- [x] 1.1 Add the ordered current-schema migration for nullable message session identity plus strict context-summary/cursor tables and verify released-v4 upgrade, schema validation, interrupted migration recovery and historical-candidate compatibility with real Dolt fixtures.
- [x] 1.2 Preserve optional session identity and summary/cursor records through full-memory export and legacy import/recovery, and verify exact output retains null legacy attribution and global sequences.
- [x] 1.3 Add explicit session-bearing typed append DTOs and route current private actor/relationship history writes through them without changing notes or transcript policy, and verify persisted rows carry the invocation session while compatibility writes remain unattributed.

## 2. Snapshot and checkpoint contract

- [x] 2.1 Implement row- and byte-bounded streaming actor/session/source snapshot DTOs and local live/candidate queries, and verify interleaved session sentinels, exact pinned view/revision, sequence ordering, derived `through`, invalid bounds and legacy exclusion.
- [x] 2.2 Implement the strict `context_summary.v1` DTO, typed stale-domain error and one-transaction summary/cursor mutation, and verify exact settlement plus revision, view, complete bounded range and competing-cursor rejection with no partial effect.
- [x] 2.3 Add a row- and byte-bounded cursor-selected summary projection through local/remote live/candidate views, and verify rolling summaries expose only the current identity while export retains predecessors.

## 3. Managed memory boundary

- [x] 3.1 Carry session append, snapshot and conditional checkpoint through the authenticated facade/service/RPC with bounded frames and canonical receipt fingerprints, and verify local/remote live/candidate equivalence plus definite stale reconstruction.
- [x] 3.2 Add accepted lost-reply and cancellation fixtures for the checkpoint operation, and verify exact receipt recovery commits once while an unproved outcome retains the shared mutation fence across reconnect.

## 4. Runtime integration and evidence

- [x] 4.1 Run the first-slice memory/runtime typechecks and real-Dolt acceptance groups in the package-owned targets, record exact observed results in `verification.md`, and keep later runtime, combined coverage and hosted evidence pending until it actually runs.
- [x] 4.2 Confirm the existing conversation-driver lease and ordinary admission fixtures remain unchanged, and document that only P30 concurrent conversation-driver admission remains deferred.
- [x] 4.3 Reconcile the P09 blocker against the delivered typed DTOs and exact change slug, run strict Cospec validation/apply/verify, and archive only after every non-deferred ledger row has observed evidence.
- [x] 4.4 Add bounded session-history reads through local/remote live/candidate views, use them for raw actor and relationship context, and verify other-session, null legacy and opaque continuation rows do not leak while policy-admitted notes/summaries remain available.
- [x] 4.5 Reload and validate one topology before each turn, retain it for that turn, and verify a committed topology change appears on the next turn without partially changing admitted work.
- [x] 4.6 Add one cancellation-safe project dream lease through local and managed remote views, and verify two dreams serialize, ordinary chat remains usable and cancellation/disconnect releases the lease.
- [x] 4.7 Verify a live-memory change during dream inference preserves the candidate/report and current history, returns an explicit conflict, publishes no stale topology and requires checked resolution without automatic rerun.
