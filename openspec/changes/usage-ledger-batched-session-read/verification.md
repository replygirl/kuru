# Verification

## 1. One keyed record read per index page [critical]

- [ ] 1.1 @benchmark (agent) `session_reads_each_index_page_with_one_keyed_record_read` against a live isolated store seeded with 300 invocations (P = 3) -> one `session()` call issues exactly 2P + 2 = 8 statements by the task-scoped counter; before the fix it fails with the observed P + 2 + N count
- [ ] 1.2 @unit (agent) the same test plants an index row whose record is absent -> `session()` refuses with `usage session index references a missing invocation`

## 2. Meaning is unchanged

- [ ] 2.1 @equivalence (agent) the same test compares the 300-invocation `session()` result with `fold_session` over the same records in index key order -> equal
- [ ] 2.2 @equivalence (agent) `mise run //packages/kuru-memory:test` -> the golden corpus tests, `session_pages_past_one_index_page_and_marks_refuse_recorded_sessions` and every existing usage-ledger test pass unchanged
- [ ] 2.3 @regression (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck` -> clean exits

## 3. Windows and Linux

- [ ] 3.1 @runtime (agent) native Windows and Linux runs of the memory test suite -> CI's native jobs on the PR
