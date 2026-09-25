## 1. Schema and typed provenance

- [x] 1.1 Add the forward main-schema v6 migration with nullable turn, operation and producer columns while retaining usage v4, and verify fresh, upgraded and reopened stores report the intended independent versions.
- [x] 1.2 Extend context-summary and private-reasoning DTO validation, serialization and identity derivation with exclusive turn/operation provenance, and verify legacy IDs/keys/JSON remain byte-identical while operation identities are domain-separated.
- [x] 1.3 Extend export and restore for both provenance kinds without rewriting historical rows, and verify a v5 fixture round-trips through v6 with exact legacy records.

## 2. Atomic owner mutation

- [x] 2.1 Add the typed combined context-checkpoint envelope and shared complete-payload validator, and verify invalid binding, exact-limit escaping and one-byte-over input before transaction work.
- [x] 2.2 Insert pre-encoded private sidecar keys inside the existing revision/range/cursor-checked context transaction, and verify success, later-record conflict and typed stale outcomes are all-or-none on real Dolt.

## 3. Managed facade and receipt

- [x] 3.1 Extend local/remote facade and service RPC dispatch with the whole envelope under the existing context-checkpoint receipt, and verify validation happens before backend selection and owner validation repeats.
- [x] 3.2 Exercise an accepted lost managed reply and exact retry, and verify one receipt proves the complete context/cursor/sidecar outcome without duplicate or prefix publication.

## 4. Gates and handoff

- [x] 4.1 Run focused store/facade/service fixtures, all-target memory typecheck, Rust formatting, strict Cospec validation and diff-check; record exact observed results in the verification ledger.
- [x] 4.2 Document the final DTO/method/migration contract and exact prerequisite commit for actor-context-compaction integration, and verify the dependent change can remove its hard blocker without copying shared files.
