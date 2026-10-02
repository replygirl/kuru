# Tasks

## 1. Regression tests first

- [x] 1.1 Add kuru-platform `RemovalError::descendant` as an always-`None` field, then add the Unix `checked_tree_removal_names_the_refusing_nested_descendant` test and the Windows `checked_tree_removal_names_a_held_nested_descendant` test, and verify the Unix test is red on macOS
- [x] 1.2 Add the cross-OS `provision/sweep_tests.rs` module (moving the five direct-sweep tests), with `sweep_refusal_rewrites_only_the_refusal_fields`, `sweep_that_collects_the_last_receipt_removes_leftovers`, `a_refused_leftovers_removal_is_reported` and `warm_sweep_reports_a_failed_lock_attempt` among its tests, and verify each named regression test is red on macOS
- [x] 1.3 Rewrite the Windows concurrent cold-provision test to the contract with the cause table (`retried_cleanup_classes_match_the_bounded_recovery_table`) and stderr evidence, and add the Windows probe-child and process-identity tests, and verify they compile through root `lint:windows`

## 2. Fix

- [x] 2.1 Thread the descendant cursor through both native `remove_tree` bodies, and verify the 1.1 Unix test is green
- [x] 2.2 Add `ProcessStamp`, `NativeChild::stamp` and `process_object_retained` to kuru-platform's Windows process module, and verify that Windows lint is clean
- [x] 2.3 Record the probe child's stamp on the stage lease, and the descendant and probe-child state in the receipt, and verify the report and receipt tests
- [x] 2.4 Record sweep refusals in the receipt, remove an empty `.leftovers`, and report a refused folder removal, a failed receipt rewrite and a failed warm lock attempt, and verify the 1.2 tests are green
- [x] 2.5 Update docs/memory.md for collection, refusal recording and the receipts folder, and verify `docs:check`

## 3. Verification

- [x] 3.1 Run `mise run //packages/kuru-memory:test` and `mise run //packages/kuru-platform:test`, and record the observed results
- [x] 3.2 Run `mise run format:check`, `mise run lint`, root `mise run lint:windows`, `mise run typecheck` and `mise run docs:check`, and record the exits
- [x] 3.3 Confirm the "must not change" list holds in the diff, and verify by reading it: `LOCK_TIMEOUT`, `CLEANUP_RETRY_LIMIT`/`SPACING`, `ACTIVATION_RETRY_LIMIT`, one attempt per receipt per sweep, the receipt and diagnostic before lock release, no deletion on uncertainty, warm opens never waiting, `LEFTOVER_STAGE_CAP` reporting only, private-object validation, `remove_tree`'s owner-private requirement, `StagedActivation` drop order, the unsafe-code boundary, and receipt `version: 1`
