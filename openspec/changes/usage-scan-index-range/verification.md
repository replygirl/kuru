# Verification

## 1. Owned-state pages read a primary-key range [critical]

- [ ] 1.1 @integration (agent) plan test against a live isolated store: each rewritten query string (owned first page, owned later page, session index first and later page, the `mark_new_session` probe) is bound through sqlx exactly as production binds it, and `EXPLAIN PLAN` is read for it -> the plan has `IndexedTableAccess(state)` on `index: [state.key]` with the bound byte range, and has neither `TopN` nor a bare `Table` node; the old query, bound the same way, plans as `TopN` over a full `Table` (negative control)
- [ ] 1.2 @unit (agent) `prefix_upper_bound` unit tests -> trailing `/` becomes `0`, a trailing 0xFF carries into the previous byte and is dropped, an all-0xFF or empty prefix has no bound
- [ ] 1.3 @benchmark (agent) fresh APFS clones of the unit 6b aged stores (1k / 5k / 20k conversations), pinned Dolt 2.3.5 `sql-server`, one full owned-state walk through sqlx with the old and the new page query, SQL and decode timed separately -> the new walk is linear in rows and its pages stay under 1 ms; record the before and after numbers

## 2. Meaning is unchanged

- [ ] 2.1 @equivalence (agent) ordering test over a store seeded with boundary keys (`kuru.usage.v1`, `kuru.usage.v1.x`, `kuru.usage.v10`, `kuru.usage.v10x`, the bare prefix, keys with 0x00, 0x7f, 0x80 and 0xff bytes, a trailing space, a 1024-byte key), at 129 and 257 owned rows and again with a session index spanning three pages -> the new owned and session-index walks return exactly the old query's key sequence, which equals the seeded keys that start with the prefix in byte order; the old `COUNT(*)` and the new probe agree for a session with index rows, one without, and one whose only neighbours sit at the range edges
- [ ] 2.2 @equivalence (agent) `mise run //packages/kuru-memory:test` -> the `usage-ledger-guarantee-tests` reopen refusals and boundary-key tests, and every existing usage-ledger test, pass unchanged
- [ ] 2.3 @unit (agent) `mark_new_session` on a session that already has an invocation -> refuses with `usage session already has invocation records`; `session()` folds a session with more than 128 index rows
- [ ] 2.4 @regression (agent) `mise run format:check`, `mise run lint`, `mise run //packages/kuru-memory:lint:windows`, `mise run typecheck` -> clean exits

## 3. Windows and Linux

- [ ] 3.1 @runtime (agent) native Windows and Linux runs of the memory test suite -> run by CI's native jobs on the PR; not run locally
