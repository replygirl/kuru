# Verification

## 1. Publication record rides the attempt commit and reaches main only with the fast-forward [critical]

- [ ] 1.1 @integration (agent) run `publication_record_is_in_the_attempt_commit_and_reaches_main_with_the_fast_forward` against a real Dolt engine: inspect the attempt branch's commit for its row before `publish` runs, then confirm the row appears on `main` only after `publish` returns -> row absent from main pre-publish, present post-publish, in the same commit as the schema/receipt.
- [ ] 1.2 @integration (agent) run `v8_backfills_every_published_branch_from_classification` on a real store carrying v2..v7 attempt branches -> exactly one publication row per published branch, none for a dirty or failed attempt.

## 2. A later open verifies a recorded branch by record, opening no branch or commit pool [critical]

- [ ] 2.1 @integration (agent) run `recorded_branches_open_no_branch_or_commit_pools` with a counting hook on `Server::pool` across a classification pass where every retained branch is recorded -> pool-open count for branches/commits is zero.
- [ ] 2.2 @integration (agent) run `unrecorded_failed_attempt_is_still_fully_classified` -> an unrecorded branch still receives today's full classification unchanged.

## 3. Every record mismatch fails the open closed, without mutation and without fallback [critical]

- [ ] 3.1 @integration (agent) run `moved_recorded_ref_fails_closed_without_mutation`, `dirty_recorded_branch_fails_closed`, `record_outside_main_history_fails_closed`, `as_of_receipt_mismatch_fails_closed` -> each fails the open with the disagreement reported, no branch/record/main mutation observed afterward.
- [ ] 3.2 @integration (agent) run `validate_attempt_rejects_record_that_disagrees_with_name_base_receipt_or_digest` -> the in-progress attempt fails validation before publication.
- [ ] 3.3 @integration (agent) run `reused_completed_v8_attempt_with_disagreeing_backfill_fails_closed` -> a discovered completed V8 attempt whose backfill disagrees with current classification fails closed without mutating main or the attempt branch.
- [ ] 3.4 @integration (agent) run `recorded_branch_deleted_is_tolerated` -> a record whose branch no longer exists is accepted as long as its base is still in main's history.

## 4. One-time upgrade and read-only refusal for a pre-V8 store [critical]

- [ ] 4.1 @integration (agent) run `v7_store_upgrades_once_with_two_starts_then_uses_records` with `test_support::engine_ledger` counting starts -> exactly 2 starts on the upgrading open, and zero full classifications on the following open (records used instead).
- [ ] 4.2 @integration (agent) run `read_only_open_of_v7_store_requires_writable_upgrade` -> unchanged existing refusal message and behavior, no migration, no mutation.
- [ ] 4.3 @integration (agent) run `ready_v7_stage_from_previous_binary_is_classified_then_upgraded` -> a ready v7 stage from a prior binary is classified in full at its own version, then activated and upgraded.

## 5. Schema authority and template-shape check treat the publications table as authority

- [ ] 5.1 @integration (agent) run `publication_table_change_is_a_schema_authority_violation` -> a working change to `kuru_migration_publications` is rejected the same way a change to `kuru_schema`/`kuru_migrations` is.
- [ ] 5.2 @integration (agent) build (or reuse the existing fixture build of) a store template under this change and run its shape check -> passes with publication records present and verified, not flagged as stray project data.

## 6. Parity: record-based verdicts equal full-classification verdicts [critical]

- [ ] 6.1 @equivalence (agent) run `template_born_store_verdicts_match_full_classification` (or the extended existing P2 parity test) across every retained-branch state the existing parity coverage exercises (clean completed, dirty failed, deleted branch, branch above current version) -> record-based and full-classification verdicts agree in every case.

## 7. Test-only migration fixture renumbered without regressions

- [ ] 7.1 @unit (agent) `cargo test -p kuru-memory` after renaming the test-only V8 fixture to V9 -> full test suite compiles and passes under the new numbering, no leftover reference to the old test-marker-v8 id or table name.

## 8. Static and spec gates

- [ ] 8.1 @unit (agent) `mise run lint:rust` / `mise run typecheck` (or package-scoped equivalents) on touched crates -> clean.
- [ ] 8.2 @unit (agent) `mise run cospec -- validate memory-migration-publication-records --strict` -> passes.
- [ ] 8.3 @manual (human) review of amended `openspec/specs/versioned-memory/spec.md` text against actual shipped behavior before archive -> confirmed matching, or spec delta updated to match.
