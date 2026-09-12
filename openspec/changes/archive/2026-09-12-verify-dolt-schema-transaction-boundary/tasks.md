## 1. Native transaction fixtures

- [x] 1.1 In `packages/kuru-memory/src/store/recovery_tests.rs`, add bounded
  test-local helpers that execute and inspect a version/receipt-style schema
  transaction and isolated Dolt branch publication against the real pinned
  engine without changing production schema, fault switches, dependencies, or
  runtime behavior.
- [x] 1.2 Add
  `manual_dolt_commit_atomically_publishes_ddl_version_and_receipt` and verify,
  after the accepted connection is closed and its server session disappears,
  that `CREATE TABLE`, version-style DML, receipt-style DML, and the manual
  `DOLT_COMMIT` are all visible `AS OF 'HEAD'`, `dolt_status` is empty, the
  prior main HEAD is retained as the commit parent, and a pre-existing
  candidate ref and candidate history are unchanged.
- [x] 1.3 Add
  `dropping_precommit_ddl_session_retains_dirty_working_ddl_outside_head` and
  verify that dropping the connection after transactional DDL and DML but
  before `DOLT_COMMIT`, then waiting for independent observation of session
  teardown, leaves the original HEAD and history, no probe table or receipt at
  HEAD, schema version 1 at HEAD, the exact candidate ref unchanged, and exactly the observed
  unstaged new-table row in `dolt_status`. Perform no reset or cleanup write.
- [x] 1.4 Add
  `lost_manual_dolt_commit_reply_reconciles_one_clean_schema_commit` by targeting
  `CALL DOLT_COMMIT` with the existing MySQL packet fault harness and verify the
  fixture discards an actual durable reply, waits for the original session to
  disappear, observes the complete DDL/version/receipt state `AS OF 'HEAD'`,
  finds an empty `dolt_status`, preserves the prior HEAD and candidate history,
  and produces exactly one migration-style commit.
- [x] 1.5 Add
  `isolated_schema_retry_keeps_main_clean_and_reconciles_lost_fast_forward_reply`
  and verify a reserved test branch made from the exact main base retains its
  dirty DDL after pre-commit session loss while active main remains clean v1;
  preserve that failed branch, build a fresh branch from the same base, validate
  its complete clean v2 commit and direct parent, then discard an actual
  `DOLT_MERGE(..., '--ff-only')` reply and reconcile main to exactly that target
  once. Assert the old main ancestor, every source row, and the pre-existing
  candidate name, head and branch-local history remain unchanged, without any
  reset, deletion or production recovery helper.

## 2. Verification

- [x] 2.1 Run
  `mise run //packages/kuru-memory:test -- store::recovery_tests` and record the
  four named real-engine cases passing on the current native host with no
  skipped fixture.
- [x] 2.2 Verify the new cases are included in the existing supported native
  Unix and Windows test jobs; record observed host results and explicitly unrun
  platforms. Native Windows execution remains required before merge; a compile,
  mock engine, or host-only pass does not establish Windows acceptance.
- [x] 2.3 Run `mise run //packages/kuru-memory:typecheck` and
  `mise run //packages/kuru-memory:lint`, then strict-validate the change and
  preserve the observed direct-main dirty-DDL result as explicit regression
  evidence rather than resetting data or describing it as rollback.

## Observed evidence

2026-09-12, native macOS: the final focused recovery task passed all 10 cases
(four new, six existing; 50 filtered), exit 0, in
`/private/tmp/kuru-schema-transaction-final-tests-native.log`. Package typecheck
and lint also exited 0; their final logs share the same prefix. Root checked
the named results and explicit exits. Independent Sol review accepted the final
source, including exact merge-session teardown, reconciliation returning
`Some(true)` then `None`, clean main before publication, and preserved failed
branch, source rows and candidate history afterward.

The direct-main precommit case deliberately preserves the observed limitation:
HEAD remains v1 with no committed probe table or receipt, while the working set
retains exactly the unstaged new-table row. The earlier failed rollback premise
is retained in private recovery-test logs; no reset or cleanup conceals it.
The positive retry case creates exact-base test branches and publishes only the
validated clean v2 target after an actual lost fast-forward reply.

The existing native-tests CI matrix includes Ubuntu, macOS and Windows and its
coverage suite includes these unconditionally compiled recovery tests. Native
Windows execution is unrun locally and remains required before merge. The
initial restricted fixture attempt could not bind loopback; the final native
run above executed every recovery case without skips. Strict validation passed
with zero errors or warnings.

This establishes engine primitives only. Production migration startup, cold
attempt discovery, branch-creation reply loss, cancellation ownership and
staging recovery remain implementation and verification work for the separate
ordered-migration change.

The final single workspace coverage run also passed these four cases on native
macOS, with explicit exit 0 (session 91931), 17,781/18,415 lines covered
(96.56 percent). Its private log is
`/private/tmp/kuru-phase0-provider-diagnostics-coverage.log`. This is additional
integration evidence for the test change, not evidence that a production
migration runner already exists.
