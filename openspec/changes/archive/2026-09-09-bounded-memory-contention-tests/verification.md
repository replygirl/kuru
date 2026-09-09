## 1. Durable concurrency [critical]

- [x] 1.1 @regression (agent) Run independent_handles_coordinate_durable_writes -> Linux run 34403343728 failed before correction with SQLITE_BUSY at memory.rs:295; corrected test passes with all eighty unique records in per-writer order after reopening, joining both outcomes before assertions
- [x] 1.2 @integration (agent) Hold a real external write lock through test-local busy deadlines -> all four operations report DatabaseBusy after their isolated 25ms wait budget, restore autocommit and preserve every seed value; releasing the lock permits exactly one append, correct single/batch state updates and namespace-local deletion, verified after reopening; all 44 core tests and strict package Clippy pass
- [x] 1.3 @integration (agent) Run full mise check -> exit 0 on 2026-09-09 with 215 passing Rust tests and 97.57% line coverage (8754/8972), above the unchanged 90% gate; Clippy, formatting, tooling lint, docs and cospec checks pass

## 2. Hosted verification

- [~] 2.1 @runtime (agent) Observe checks for the corrected commit -> defer: this record is archived before commit; required Linux/macOS CI results are recorded by GitHub and reported separately before merge
