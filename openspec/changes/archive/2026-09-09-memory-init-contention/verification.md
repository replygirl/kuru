## 1. Initialization under contention [critical]

- [x] 1.1 @regression (agent) hold a real SQLite lock across WAL initialization -> the original extracted transition failed immediately with SQLITE_BUSY; the corrected transition stayed pending while BEGIN IMMEDIATE was held, succeeded after release, retained the seeded row and selected WAL
- [x] 1.2 @integration (agent) exercise persistent contention and a non-busy SQLite error -> a 20ms private deadline returns contextual SQLITE_BUSY with its original code; a corrupt file returns NotADatabase without waiting or altering bytes; both restore the normal five-second busy timeout
- [x] 1.3 @integration (agent) open and write from independent processes plus simultaneous threads -> four child processes coordinated at a start barrier retain all four distinct messages; the existing eight-thread first-open test passes unchanged

## 2. Existing storage contracts [critical]

- [x] 2.1 @regression (agent) run corrupt/foreign/future/malformed database tests -> all existing rejection tests pass, including preserved foreign data and its unchanged DELETE journal policy
- [x] 2.2 @integration (agent) run core tests, strict Clippy, formatting and coverage -> mise run check exited 0 on 2026-09-09 with 209 Rust tests and 97.51% line coverage (8554/8772), above the unchanged 90% threshold; all lint, format, documentation and cospec checks pass

Observed before the fix: `mise run //packages/kuru-core:test -- wal_transition_waits_for_reserved_lock_and_preserves_data` failed in 0.00s with “WAL transition returned while a real competing write lock was held” and SQLite error code 5. The regression first independently obtains SQLITE_BUSY from the real pragma while the competing reservation is held.

Observed after the fix: `mise run //packages/kuru-core:test` passed 43 tests (4 unit, 15 configuration, 7 framework, 17 memory). `mise run //packages/kuru-core:lint`, `mise exec -- cargo fmt --all -- --check` and `git diff --check` passed. Four meaningful cases were added; no existing concurrency test was weakened. The complete repository and coverage gate then passed as recorded above.
