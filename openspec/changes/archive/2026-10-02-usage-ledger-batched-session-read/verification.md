# Verification

## 1. One keyed record read per index page [critical]

- [x] 1.1 @benchmark (agent) `session_reads_each_index_page_with_one_keyed_record_read` against a live isolated store seeded with 300 invocations (P = 3), statements counted by a `#[cfg(test)]` `tokio::task_local!` counter incremented once in each pool-level read helper `session()` calls (`read_marker`, `session_index_page`, `read_state`, `read_states`), each of which issues exactly one statement; the scope is per task, so parallel tests calling the same helpers do not share it -> before the fix (commit ba4f4b5c on main 7c9581b0) observed failing with `session() over 300 invocations in 3 index pages issued 305 reads, expected 8` (P + 2 + N); after the fix (cebd39b1) observed passing with exactly 8 (2P + 2), 2026-10-02 on macOS arm64
- [x] 1.2 @integration (agent) `bound_range_queries_plan_as_primary_key_ranges` reads `EXPLAIN PLAN` for the keyed read of three bound record keys on the pinned Dolt -> `IndexedTableAccess(state)` on `index: [state.key]` with one `[k, k]` point range per bound key and no bare `Table` node; observed passing 2026-10-02
- [x] 1.3 @unit (agent) the same benchmark test plants an index row whose record is absent -> `session()` refuses with `usage session index references a missing invocation`; observed passing 2026-10-02

## 2. Meaning is unchanged

- [x] 2.1 @equivalence (agent) the benchmark test compares the 300-invocation `session()` result with `fold_session` over the same records in index key order -> equal; observed passing before and after the fix 2026-10-02 (the red run failed only at the count assertion, after the equality check)
- [x] 2.2 @equivalence (agent) `mise run //packages/kuru-memory:test` -> exit 0 in 841 s, 2026-10-02 on macOS arm64 (lib 694 passed, 6 ignored; bundle_build 10, memory 5, server_lifecycle 12, supervisor_snapshot 1 passed); `golden_corpus_pins_the_validator_verdicts`, `session_pages_past_one_index_page_and_marks_refuse_recorded_sessions` and every other usage-ledger test passed unchanged
- [x] 2.3 @regression (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck` -> exit 0 for each, 2026-10-02 on macOS

## 3. Windows and Linux

- [~] 3.1 @runtime (agent) native Windows and Linux runs of the memory test suite -> defer: no local Windows or Linux host; CI's native jobs run the suite on the PR. Windows-target clippy (2.3) is the only local Windows evidence
