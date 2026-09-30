# Tasks

## 1. Settle the parent-schema validation form

- [ ] 1.1 On a real Dolt 2.3.5 store with at least one retained migration branch, try (in order) the detached `USE kuru/<hash>` form on a main-pool connection with the existing `validate_version_with`/`validate_schema_with` queries unmodified, then `SHOW CREATE TABLE ... AS OF`, then `information_schema` with `AS OF` or a revision-qualified database name, for both `root` and `kuru_reader`, against a valid parent and a parent with a deliberately wrong schema (an injected extra/missing column or table) — and verify by exact-match verdict comparison against `validate_version_with` on the branch pool for both the positive and negative case.
- [ ] 1.2 Record which form was chosen, why, and the verdicts observed for the rejected forms in `tmp/roadmap/store-creation-design/engine-contract-findings.md` (append, do not rewrite) or a new dated findings file in the same directory — and verify the file exists and names the chosen form's exact query text.
- [ ] 1.3 If no form reproduces `validate_version_with`'s verdict exactly for both roles and both the positive and negative case, stop here: do not proceed to task 2, record the finding, and report `ok=false` with the reason — and verify by the findings file stating this outcome explicitly rather than by silence.

## 2. Write the main-pool classifier

- [ ] 2.1 Write a new classification function that answers every check `classify_historical_attempts_in` makes today (attempt revision via `dolt_branches.hash`; `kuru_schema` version via `AS OF`; dirty state via `dolt_branches.dirty` cross-checked against `` `kuru/<branch>`.dolt_status ``; receipt row match by UUID via `kuru_migrations AS OF`; sole parent via `dolt_commit_ancestors`; parent schema validation using the form chosen in task 1; ancestry of head and parent via `dolt_log`) using only the caller's existing main `MySqlPool` parameter — no `server: &Server` parameter, no `Server::pool` call anywhere in its body — and verify by code inspection (no `server.pool(` call in the new function) plus a passing compile.
- [ ] 2.2 Make every new query fail closed (`ensure!`/equivalent) on a missing row, an unexpected row count, or a mismatched column, exactly as today's code fails closed on an inconsistent branch — and verify with a unit test per new query that feeds it a row shape that should not occur and asserts the branch is rejected, not silently skipped or accepted.
- [ ] 2.3 Add the `malformed_branch_hash_never_reaches_revision_sql`-style guard (existing pattern at `store.rs:11814`) for every new hash/commit value interpolated into `AS OF` or a database-qualified identifier, so an unvalidated string never reaches the engine — and verify with the new `malformed_branch_hash_never_reaches_revision_sql` test in this change's suite.

## 3. Prove parity with the old pool-based classifier

- [ ] 3.1 Build fixtures for each retained-branch state named in the brief: clean published, dirtied working set, ref force-moved to another commit, receipt row removed or mismatched, parent with a wrong schema, and a branch whose instance identity row differs from main's (produced as the S8 contract test does) — and verify each fixture is independently confirmed to be in the intended state before the comparison (e.g., the dirty fixture reads `dolt_status` rows, the moved-ref fixture reads a different `dolt_branches.hash` than before the move).
- [ ] 3.2 Keep the current pool-based `classify_historical_attempts_in` reachable as a test-only oracle (under `#[cfg(test)]`, not from any product call site) if needed for the comparison, and say so in the PR description — and verify by `cfg(test)` gating plus no product call site referencing it.
- [ ] 3.3 Write `main_pool_classification_agrees_with_branch_pool_classification`: for every fixture in 3.1, assert the new classifier and the old pool-based classifier return the same verdict (both pass, or both fail with an equivalent reason) — and verify the test passes locally against the pinned engine.
- [ ] 3.4 If parity cannot be shown for any fixture state, stop: do not proceed to task 4, record the divergent state and the observed difference in the findings file from task 1.2, and report `ok=false` with the reason (the brief: "the delivery order then changes") — and verify by the findings file naming the specific state and query that diverged.

## 4. Swap the call sites

- [ ] 4.1 Replace the `classify_historical_attempts_in` call in `validate_active`, `validate_inspection`, `validate_ready`, and the per-step loop in `upgrade_in` with the new main-pool classifier, removing the `server: &Server` threading that only existed for the old classifier's pool opens where nothing else in the call chain still needs it — and verify by `mise run //packages/kuru-memory:typecheck` and `mise run //packages/kuru-memory:lint`.
- [ ] 4.2 Add `historical_classification_opens_no_branch_or_commit_pools`: a counting hook on `Server::pool` (via `test_support::engine_ledger`) asserting 0 pool opens for clean retained branches on `validate_active`, `validate_inspection` (reader), and `validate_ready` — and verify the test passes and specifically fails (red) against the pre-change code path if run before 4.1, confirming it exercises the right code.
- [ ] 4.3 Update every existing test that counts pools or classification queries to the new counts (the old "2 pools per clean branch, 1 per dirty branch" expectation becomes 0) — and verify by `mise run //packages/kuru-memory:test -- migrations` passing with the updated counts and no other assertion changed.
- [ ] 4.4 Confirm every test asserting a classification *verdict* (not a pool/query count) is unchanged and still passes — and verify with a diff review of the test file showing no verdict-assertion lines touched, plus a green run.

## 5. Coverage, lint and docs

- [ ] 5.1 Run `mise run coverage` for `kuru-memory` and confirm the 90% workspace line gate still holds without excluding the new classifier or lowering the threshold — and verify by the coverage report's line percentage for the changed file(s).
- [ ] 5.2 Run `mise run //packages/kuru-memory:lint:windows` and confirm the new code is lint-clean for the Windows target (no `cfg(windows)`-only issue introduced) — and verify by a clean exit.
- [ ] 5.3 Check whether `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`, or any development doc describes classification's *mechanism* (not just its cost) in a way this change makes stale, and update only what changed — and verify by `mise run docs:check` passing and a one-line note in the PR description naming which doc lines (if any) changed and why.
- [ ] 5.4 Confirm `openspec/specs/versioned-memory/spec.md:188-189` ("branches are classified from their committed receipt, registered step and ancestry") remains true as written, with no spec delta needed — and verify by re-reading that line against the shipped classifier and stating in the PR description that it still holds, or filing the delta if task 1.3 or 3.4 changed the outcome.

## 6. Benchmark evidence

- [ ] 6.1 Record the after number for the proposal's Benchmarks table (`Server::pool` opens per classified retained branch) from the passing `historical_classification_opens_no_branch_or_commit_pools` test output, and fill in the "After (to record)" column with the observed value — and verify the proposal.md table cell is no longer a placeholder and cites the test name as its source.
