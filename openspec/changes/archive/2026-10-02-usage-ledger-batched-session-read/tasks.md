# Tasks

## 1. Red

- [x] 1.1 Add a test-only task-scoped statement counter to the read helpers `session()` uses, and a test over 300 seeded invocations asserting the fold equals the in-memory fold and exactly 2P + 2 statements per call, and verify it fails on main with the observed before count -> observed failing 2026-10-02 on main 7c9581b0: `session() over 300 invocations in 3 index pages issued 305 reads, expected 8`

## 2. Batched record read

- [x] 2.1 Add a file-local keyed read of at most `PAGE_SIZE` record keys under `QUERY_TIMEOUT` with its own deadline context, and verify it is counted once per call -> `read_states` with `keyed_read_sql`, context `usage ledger session records deadline exceeded`; `EXPLAIN PLAN` on the pinned Dolt is `IndexedTableAccess(state)` on `index: [state.key]` with one `[k, k]` point range per bound key, asserted in `bound_range_queries_plan_as_primary_key_ranges`
- [x] 2.2 Rewire `session()` to validate a page's index rows, read their records in one keyed statement and fold in index order, and verify the error texts and fold order are unchanged in the diff -> the three `ensure!`/`context` texts are untouched lines in the diff; records are looked up in the page's key order
- [x] 2.3 Run the benchmark test and record the after statement count in the proposal and verification -> `session_reads_each_index_page_with_one_keyed_record_read` passes with exactly 8 statements for 300 invocations (2026-10-02)

## 3. Evidence

- [x] 3.1 Run `mise run //packages/kuru-memory:test`, `format:check`, `lint`, `lint:windows`, `typecheck` and `cospec validate --all --strict`, and record the exit codes -> memory test exit 0 (841 s, lib 694 passed), format:check 0, lint 0, lint:windows 0, typecheck 0, cospec validate --all --strict 0 (2026-10-02)
