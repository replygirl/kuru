# Verification

## 1. No pool opened on any retained branch or its parent during classification [critical]

- [ ] 1.1 @benchmark (agent) run `historical_classification_opens_no_branch_or_commit_pools` (tasks.md 4.2), a counting hook on `Server::pool` via `test_support::engine_ledger`, over `validate_active`, `validate_inspection` (reader), and `validate_ready` on a store with clean retained branches -> 0 pool opens for the classification pass; record the observed count in proposal.md's Benchmarks table (tasks.md 6.1)
- [ ] 1.2 @integration (agent) run the same hook with a dirtied retained branch present -> 0 pool opens for the classification pass itself; the working-set shape check (`retained_failed_shape`) may still pool per design.md D1/SD §7.2, and the test records whether it does

## 2. Classification verdicts are unchanged (equivalence with today's pool-based classifier) [critical]

- [ ] 2.1 @equivalence (agent) run `main_pool_classification_agrees_with_branch_pool_classification` (tasks.md 3.3) over every fixture state named in the brief: clean published, dirtied working set, ref force-moved to another commit, receipt row removed or mismatched, parent with a wrong schema, and a branch whose instance identity differs from main's -> new classifier and old pool-based classifier return the same verdict for every fixture; a divergence on any fixture halts the change per tasks.md 3.4, not a silent pass
- [ ] 2.2 @regression (agent) run the full existing `packages/kuru-memory` migrations test suite (`mise run //packages/kuru-memory:test -- migrations`) after the call-site swap -> every test asserting a classification verdict (as opposed to a pool/query count) passes unchanged, confirming behavior did not drift

## 3. Parent-schema validation form is exact, not weakened

- [ ] 3.1 @manual (agent) execute task 1's ordered trial (`USE kuru/<hash>`, then `SHOW CREATE TABLE ... AS OF`, then `information_schema`) against a valid parent and a parent with an injected wrong schema, for both `root` and `kuru_reader` -> the chosen form's verdict matches `validate_version_with` on the branch pool for both the positive and the negative case; the findings file (tasks.md 1.2) records the comparison

## 4. Fail-closed behavior on malformed or unexpected input

- [ ] 4.1 @unit (agent) unit tests per new query (tasks.md 2.2) feeding a row shape that should not occur (no row, extra row, mismatched column) -> the branch is rejected as ambiguous, never treated as published
- [ ] 4.2 @unit (agent) `malformed_branch_hash_never_reaches_revision_sql` (tasks.md 2.3) -> an unvalidated hash/commit string never reaches an `AS OF` or database-qualified query

## 5. Coverage and lint hold

- [ ] 5.1 @runtime (agent) `mise run coverage` for the workspace including `kuru-memory` -> 90% workspace line gate holds with the new classifier included, not excluded
- [ ] 5.2 @runtime (agent) `mise run //packages/kuru-memory:lint:windows` -> clean exit, no Windows-target lint issue introduced

## 6. Documentation and spec stay accurate

- [ ] 6.1 @manual (agent) review `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`, and `openspec/specs/versioned-memory/spec.md:188-189` against the shipped classifier -> no stale mechanism description remains; spec line still holds as written, or a delta is filed if tasks.md 1.3/3.4 triggered a stop condition that changes the outcome
