# Tasks

## 1. Settle the parent-schema validation form

- [x] 1.1 On a real Dolt 2.3.5 store with at least one retained migration branch, try (in order) the detached `USE kuru/<hash>` form on a main-pool connection with the existing `validate_version_with`/`validate_schema_with` queries unmodified, then `SHOW CREATE TABLE ... AS OF`, then `information_schema` with `AS OF` or a revision-qualified database name, for both `root` and `kuru_reader`, against a valid parent and a parent with a deliberately wrong schema (an injected extra/missing column or table) — and verify by exact-match verdict comparison against `validate_version_with` on the branch pool for both the positive and negative case.
- [x] 1.2 Record which form was chosen, why, and the verdicts observed for the rejected forms in `tmp/roadmap/store-creation-design/engine-contract-findings.md` (append, do not rewrite) or a new dated findings file in the same directory — and verify the file exists and names the chosen form's exact query text.
- [x] 1.3 If no form reproduces `validate_version_with`'s verdict exactly for both roles and both the positive and negative case, stop here: do not proceed to task 2, record the finding, and report `ok=false` with the reason — and verify by the findings file stating this outcome explicitly rather than by silence.

## 2. Write the main-pool classifier

- [x] 2.1 Write a new classification function that answers every check `classify_historical_attempts_in` makes today (attempt revision via `dolt_branches.hash`; `kuru_schema` version via `AS OF`; dirty state from the branch-qualified `` `kuru/<branch>`.dolt_status `` (the same table the branch pool read; `dolt_branches.dirty` is deliberately not cross-checked, see evidence); receipt row match by UUID via `kuru_migrations AS OF`; sole parent via `dolt_commit_ancestors`; parent schema validation using the form chosen in task 1; ancestry of head and parent via `dolt_log`) using only the caller's existing main `MySqlPool` parameter — no `server: &Server` parameter, no `Server::pool` call anywhere in its body — and verify by code inspection (no `server.pool(` call in the new function) plus a passing compile.
- [x] 2.2 Make every new query fail closed (`ensure!`/equivalent) on a missing row, an unexpected row count, or a mismatched column, exactly as today's code fails closed on an inconsistent branch — and verify with a unit test per new query that feeds it a row shape that should not occur and asserts the branch is rejected, not silently skipped or accepted.
- [x] 2.3 Add the `malformed_branch_hash_never_reaches_revision_sql`-style guard (existing pattern at `store.rs:11814`) for every new hash/commit value interpolated into `AS OF` or a database-qualified identifier, so an unvalidated string never reaches the engine — and verify with the new `malformed_branch_hash_never_reaches_revision_sql` test in this change's suite.

## 3. Prove parity with the old pool-based classifier

- [x] 3.1 Build fixtures for each retained-branch state named in the brief: clean published, dirtied working set, ref force-moved to another commit, receipt row removed or mismatched, parent with a wrong schema, and a branch whose instance identity row differs from main's (produced as the S2 contract test does) — and verify each fixture is independently confirmed to be in the intended state before the comparison (e.g., the dirty fixture reads `dolt_status` rows, the moved-ref fixture reads a different `dolt_branches.hash` than before the move).
- [x] 3.2 Keep the current pool-based `classify_historical_attempts_in` reachable as a test-only oracle (under `#[cfg(test)]`, not from any product call site) if needed for the comparison, and say so in the PR description — and verify by `cfg(test)` gating plus no product call site referencing it.
- [x] 3.3 Write `main_pool_classification_agrees_with_branch_pool_classification`: for every fixture in 3.1, assert the new classifier and the old pool-based classifier return the same verdict (both pass, or both fail with an equivalent reason) — and verify the test passes locally against the pinned engine.
- [x] 3.4 If parity cannot be shown for any fixture state, stop: do not proceed to task 4, record the divergent state and the observed difference in the findings file from task 1.2, and report `ok=false` with the reason (the brief: "the delivery order then changes") — and verify by the findings file naming the specific state and query that diverged.

## 4. Swap the call sites

- [x] 4.1 Replace the `classify_historical_attempts_in` call in `validate_active`, `validate_inspection`, `validate_ready`, and the per-step loop in `upgrade_in` with the new main-pool classifier, removing the `server: &Server` threading that only existed for the old classifier's pool opens where nothing else in the call chain still needs it — and verify by `mise run //packages/kuru-memory:typecheck` and `mise run //packages/kuru-memory:lint`.
- [x] 4.2 Add `historical_classification_opens_no_branch_or_commit_pools`: a counting hook on `Server::pool` (a `cfg(test)` request log on `ServerInner`; `test_support::engine_ledger` has no such hook) asserting 0 pool opens for clean retained branches on `validate_active`, `validate_inspection` (reader), and `validate_ready` — and verify the test passes and that the hook counts the pre-change path (the kept oracle on the same server requests 2 per clean retained branch).
- [x] 4.3 Update every existing test that counts pools or classification queries to the new counts (the old "2 pools per clean branch" expectation becomes 0) — and verify by `mise run //packages/kuru-memory:test` passing with the updated counts and no other assertion changed.
- [x] 4.4 Confirm every test asserting a classification *verdict* (not a pool/query count) is unchanged and still passes — and verify with a diff review of the test file showing no verdict-assertion lines touched, plus a green run.

## 5. Coverage, lint and docs

- [ ] 5.1 Run `mise run coverage` for `kuru-memory` and confirm the 90% workspace line gate still holds without excluding the new classifier or lowering the threshold — and verify by the coverage report's line percentage for the changed file(s).
- [x] 5.2 Run `mise run //packages/kuru-memory:lint:windows` and confirm the new code is lint-clean for the Windows target (no `cfg(windows)`-only issue introduced) — and verify by a clean exit.
- [x] 5.3 Check whether `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`, or any development doc describes classification's *mechanism* (not just its cost) in a way this change makes stale, and update only what changed — and verify by `mise run docs:check` passing and a one-line note in the PR description naming which doc lines (if any) changed and why.
- [x] 5.4 Confirm `openspec/specs/versioned-memory/spec.md:188-189` ("branches are classified from their committed receipt, registered step and ancestry") remains true as written, with no spec delta needed — and verify by re-reading that line against the shipped classifier and stating in the PR description that it still holds, or filing the delta if task 1.3 or 3.4 changed the outcome.

## 6. Benchmark evidence

- [x] 6.1 Record the after number for the proposal's Benchmarks table (`Server::pool` opens per classified retained branch) from the passing `historical_classification_opens_no_branch_or_commit_pools` test output, and fill in the "After (to record)" column with the observed value — and verify the proposal.md table cell is no longer a placeholder and cites the test name as its source.

## Observed evidence

Recorded 2026-09-30 on macOS 27.0 arm64, debug profile, pinned Dolt 2.3.5, a
shared machine under heavy load. Full detail:
`tmp/roadmap/store-creation-design/p2-main-pool-classification-findings.md`.

- 1.1-1.3: the first form tried, a detached `USE \`kuru/<hash>\`` session from
  the caller's pool running the unchanged validator, reproduced the branch-pool
  verdict exactly for root and `kuru_reader`: `Ok` for every valid parent, and
  for a parent whose defect is visible only through `information_schema`
  (`context_summaries.summary_namespace` narrowed to `VARBINARY(512)`) both
  classifiers returned `context_summaries column summary_namespace differs
  from schema v5`. Forms 2 and 3 were not needed (form 3 was already measured
  by PR #136's S8 to return 0 columns from main). No stop condition.
- 2.1: `classify_historical_attempts_in(registry, main, current, prefix)` has
  no `Server` parameter. The dirty discriminator is the branch-qualified
  `dolt_status` count, the table the branch pool read; `dolt_branches.dirty`
  is not cross-checked because an `ensure!` on their agreement would add a
  failure mode today's code does not have. Check order is unchanged.
- 2.2: `main_pool_reads_fail_closed_on_missing_rows` passed: an absent branch
  (no `dolt_branches` row, no qualified `dolt_status`), an absent commit
  (`AS OF`, `USE`), an absent receipt row, and a commit validated at the wrong
  version each return an error.
- 2.3: `malformed_branch_hash_never_reaches_revision_sql` passed against an
  unreachable pool: malformed hashes fail with the guard's message before any
  SQL and before the revision session is acquired; malformed branch names are
  refused before a qualified `dolt_status` is formed; a well-formed hash does
  reach the pool (control).
- 3.1-3.4: `main_pool_classification_agrees_with_branch_pool_classification`
  passed; full verdict strings were identical for root and `kuru_reader` in
  all 12 states (table in the findings file). Each fixture was confirmed
  first (dirty flag and status rows, moved hash, advanced commit, defect side
  confirmed by commit-pool validation) and refs were re-asserted equal to the
  baseline after each removal. The identity state is
  `adopted_store_classifies_retained_branches_from_main`: the ordinary
  writable reopen of an adopted store succeeds with no migration branch or
  commit pooled, the new verdict equals the oracle's on the same branches
  before adoption (`Ok`), and the oracle on the adopted store fails with
  `identity mismatch`, which is the designed purpose of the change, asserted
  explicitly. The old classifier is kept only under `cfg(test)` as
  `classify_with_branch_pools_in`; no product call site references it.
- 4.1: `typecheck`, `lint` and `lint:windows` for `//packages/kuru-memory`
  passed. `validate_active`, `validate_inspection`, `validate_ready` and
  `validate_usage` no longer take a `Server`; `upgrade_in` keeps it for the
  current step's attempt pools.
- 4.2: `historical_classification_opens_no_branch_or_commit_pools` passed:
  writable reopen and read-only open of an existing project request no pool
  named for a migration branch or commit; direct `validate_active` and
  `validate_ready` add no request; the oracle requests 12 for 6 clean
  branches (positive control for the hook).
- 4.3-4.4: no existing test counted pools or classification queries, so no
  count changed. No verdict assertion was edited; existing call sites changed
  only by dropping the `Server` argument. Full `//packages/kuru-memory:test`:
  363 passed, 0 failed, 4 ignored in the library suite (620 s), and every
  other target of the package passed (exit 0). The new tests were re-run
  after the last test edit: 5 passed, 1 ignored (measurement).
- 5.1: not run locally. Coverage and its 90% gate are enforced in CI; the
  instrumented workspace run was not repeated on this loaded shared machine.
  The new classifier is exercised by the tests above and nothing is excluded.
- 5.2: `mise run //packages/kuru-memory:lint:windows` exited 0.
- 5.3: `docs/memory.md` and `apps/kuru-docs/concepts/memory.md` describe
  retained attempts by outcome (preserved, ambiguous stops startup), not by
  pools; no development doc describes classification. No doc changed;
  `mise run docs:check` passed.
- 5.4: `openspec/specs/versioned-memory/spec.md:188-189` still holds: the
  classifier reads the committed receipt (`AS OF` the head), the registered
  step (branch name and registry) and ancestry (`dolt_commit_ancestors`,
  `dolt_log`), and still validates a clean branch's sole parent as the step's
  source schema. No spec delta.
- 6.1: `Server::pool` requests per classified retained branch: 2 before
  (clean: branch and parent commit; dirty: branch and head commit), 0 after.
  `validate_active` on a v7 store with six retained branches, N=7, load
  average 31-40: branch-pool path median 270.2 ms, main-pool median 187.3 ms
  (report only; "before" is the kept oracle path in the same process). A
  second batch at load average 6-7: 264.4 ms and 182.6 ms (N=7 each).
