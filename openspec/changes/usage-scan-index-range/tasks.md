# Tasks

## 1. Range paging

- [x] 1.1 Add `KeyRange` and `prefix_upper_bound`, and the first-page, later-page and existence-probe query strings, and verify with the bound unit tests
- [x] 1.2 Page `validate_branch`'s owned-state scan and `session()`'s index walk by the range, with the cursor bound as bytes and the range computed once per walk, and verify decode and error messages are unchanged in the diff
- [x] 1.3 Replace `session_has_records_tx`'s `COUNT(*)` with the `SELECT 1 … LIMIT 1 FOR UPDATE` range probe and verify `mark_new_session` still refuses a session with records

## 2. Tests

- [x] 2.1 Add the ordering-equivalence test against the old query (kept in test code) over boundary keys at 129 and 257 owned rows and a three-page session index, and verify it passes
- [x] 2.2 Add the plan test over the sqlx-bound query strings with the old query as a negative control, and verify it passes and fails when a `BINARY` wrapper is restored
- [x] 2.3 Add a `session()` test over more than 128 index rows and a precise `mark_new_session` refusal assertion, and verify both pass

## 3. Evidence

- [x] 3.1 Run the benchmark on fresh clones of the aged 1k / 5k / 20k stores and record the before and after numbers in verification 1.3 and the proposal
- [x] 3.2 Run `mise run //packages/kuru-memory:test`, `format:check`, `lint`, `//packages/kuru-memory:lint:windows` and `typecheck`, and record the results
