# Verification

Windows behaviour cannot run on this macOS host. Windows-only tests are
compiled and linted through root `mise run lint:windows`, which runs clippy
with `--all-targets` for `x86_64-pc-windows-msvc` in every package that owns
the task. Their native result belongs to CI.

Two kinds of row appear below:

- **Deterministic red before the fix**: the test fails on every run without
  the fix.
- **Fails without the fix only when a stage remains**: the test depends on a
  native hold that the host does not control.

Every red and green result below was observed on macOS arm64 on 2026-10-02 in
tmp/worktrees/fix-provision-leftover-stage-at-cause. Red means commit dcccd802
(tests only); green means commit b853d15e (fix).

## 1. The concurrent cold-provision test asserts the contract [critical]

- [~] 1.1 @regression (agent) `concurrent_cold_windows_provision_publishes_one_verified_native_identity` on windows-latest coverage (expected: passes whether or not the winner's stage is retained. A retained stage is accepted only when all of the following hold: it is receipted, it is `published: true`, its first cause matches a retried-class row with `attempts` present, it records its probe child (an unstamped child fails with its recorded stamp error in the message), it records `sweep_refusals >= 1`, and it had its retained-stage report, receipted with the lock held. Also, no sweep-skipped or unrecorded-refusal record was observed, `.leftovers` remains only with a receipt or a kept-folder record, and the lock is released. Every receipt is written to stderr. The test fails without the fix only when a stage remains, as in the original failure (run 36930175874 / job 110597536882)) -> defer: Windows-native, compiled and clippy-clean through root `lint:windows` (exit 0); the native result belongs to CI
- [x] 1.2 @unit (agent) `retried_cleanup_classes_match_the_bounded_recovery_table` (cross-OS) over representative `first_cause` strings for every row, one rejected removal outside 5/32 (os error 3) and a cause without `attempts` -> every retried row maps to its name, and both other causes are refused. Observed passing on macOS

## 2. Sweep refusals are recorded and `.leftovers` follows its receipts [critical]

- [x] 2.1 @regression (agent) `sweep_refusal_rewrites_only_the_refusal_fields` (cross-OS; the refusal is a nested symlink on Unix and a nested file held without `FILE_SHARE_DELETE` on Windows) -> two sweeps leave the stage (same identity) and its receipt. Every original field stays equal, `sweep_refusals` goes 1 then 2, and `last_sweep_refusal` names phase `Rejected`, the cause, the descendant `sub/link` and an `os_error`. The receipt stays under `LEFTOVER_RECEIPT_LIMIT`. Once the blocker is released, the next sweep collects the stage and its receipt and removes `.leftovers`. Red: `no entry found for key` (`sweep_refusals`); green -> observed. The Windows variant (`sub\held`, `os_error` 32) compiles through `lint:windows`; its native result belongs to CI
- [x] 2.2 @regression (agent) `sweep_that_collects_the_last_receipt_removes_leftovers` -> red: "a sweep that collects the last receipt removes the receipts folder and its empty staging"; green -> observed
- [x] 2.3 @unit (agent) `sweep_keeps_leftovers_while_a_receipt_remains` and `sweep_keeps_leftovers_whose_staging_holds_a_record` -> `.leftovers` and a non-empty `staging` are kept. Passed both before and after the fix (guards). Observed
- [x] 2.4 @regression (agent) `a_refused_leftovers_removal_is_reported` (Unix: version directory mode 0500; Windows: `staging` held without `FILE_SHARE_DELETE`) -> `.leftovers` stays, and exactly one `kuru.memory` "receipts folder kept" record names it, with its cause and an `os_error`. Once nothing holds it, the next sweep removes it. Red: `left: 0, right: 1` (no record); green -> observed on macOS. The Windows variant (expects `os_error` 32) compiles through `lint:windows`; its native result belongs to CI
- [x] 2.5 @unit (agent) the five direct-sweep tests moved out of the Unix-only module (`sweep_collects_only_receipted_stages`, `collect_leftover_receipt_never_counts_a_receipt_it_could_not_remove`, `sweep_leaves_a_stage_and_its_receipt_when_removal_is_rejected`, `sweep_reports_the_cap_without_deleting_any_uncollectable_stage`, `sweep_leaves_an_oversized_or_unparsable_receipt_as_remaining`) -> pass on macOS (before and after the fix) and compile for Windows through `lint:windows`

## 3. Every lock acquisition sweeps first; warm opens never wait

- [x] 3.1 @unit (agent) `warm_sweep_collects_a_receipt_when_the_lock_is_free` and `warm_sweep_leaves_a_receipt_untouched_when_the_lock_is_busy` (cross-OS; `warm_sweep_if_receipted` called directly) -> a free lock collects the stage and its receipt and removes `.leftovers` (red before the fix on the folder assertion). With the lock held by the test, the receipt stays byte-identical and the stage is kept; the holder's own sweep then collects both. Observed
- [x] 3.2 @regression (agent) `warm_sweep_reports_a_failed_lock_attempt` (`.install.lock` replaced by a directory) -> exactly one "sweep skipped" record for this cache, and the stage and receipt bytes are unchanged. Red: `left: 0, right: 1`; green -> observed
- [x] 3.3 @unit (agent) Unix `cold_provision_sweeps_a_receipted_stage_before_creating_a_new_one`, `warm_open_skips_the_sweep_when_the_installation_lock_is_busy` and `exhausted_stage_cleanup_publishes_the_engine_and_receipts_the_retained_stage`, extended -> all three were red before the fix on their `.leftovers` assertions. After the fix, the cold and warm paths leave no `.leftovers` once they collect, and a busy warm open leaves the receipt byte-identical. Observed

## 4. Refusal evidence: descendant and probe child

- [x] 4.1 @regression (agent) kuru-platform `checked_tree_removal_names_the_refusing_nested_descendant` (Unix) and the extended `checked_tree_removal_rejects_a_symlink_descendant` / `checked_tree_removal_rejects_a_replaced_root_without_touching_it` -> red with `descendant` always `None`: `left: None, right: Some("sub/link")` and `Some("link")`. Green after threading the cursor (6 passed), and a root failure names no descendant
- [~] 4.2 @unit (agent) kuru-platform Windows `checked_tree_removal_names_a_held_nested_descendant` (expected: `sub\held`, phase `Rejected`, native error 32) -> defer: Windows-native; compiled through `lint:windows`; the native result belongs to CI
- [~] 4.3 @unit (agent) kuru-platform Windows `process_object_retained_matches_only_the_exact_identity` (expected: the current process is `true`. An exited child whose duplicated handle we hold is `true`, and the same id with another creation time is `false`. No control asserts `false` after the handle is dropped, because another process may legitimately hold the object. This is the positive control for the receipt's `retained` reading: if it fails natively, `process_object_retained` is not measuring what the receipt claims, and the instrument must be redesigned before any `at_refusal` value is trusted. That a terminated process stays openable by its id while a handle is open is inferred from Windows documentation and was not measured here) -> defer: Windows-native; compiled through `lint:windows`; the native result belongs to CI
- [x] 4.4 @unit (agent) `a_stage_cleanup_report_names_the_refusing_descendant` (cross-OS) -> a real `RemovalError` in the cause chain puts its descendant into the report and the receipt (`sub/link` on macOS). Red: `no entry found for key` (`descendant`); green -> observed
- [~] 4.5 @integration (agent) Windows `forced_published_cleanup_failure_records_the_probe_child` (real embedded engine, forced cleanup failure) (expected: the receipt records a probe child with non-zero `pid` and `created` and an `at_refusal` value; a failed stamp fails with its recorded `error`. The warm open then either collects the stage and its receipt, or records `sweep_refusals: 1` with `probe_child_at_refusal`. Its warm sweep runs within milliseconds of publication, so on CI it is also a cheap per-run probe of whether the family's hold is present. Its printed receipt is the first measured `at_refusal`) -> defer: Windows-native; compiled through `lint:windows`; the native result belongs to CI
- [~] 4.6 @unit (agent) Windows `cancelled_activation_recovery_receipts_a_held_stage_before_releasing_the_lock` (expected: after the holder drops, the product sweep (not `remove_dir_all`) collects the stage and its receipt (`collected == 1`, `remaining == 0`). It deliberately does not assert that `.leftovers` is gone: that removal is one best-effort attempt (critique change 5), and the cross-OS rows above pin it) -> defer: Windows-native; compiled through `lint:windows`; the native result belongs to CI
- [x] 4.7 @regression (agent) review finding: a failed probe-child stamp was dropped (`child.stamp().ok()`), leaving `probe_child: null` with no cause although the spec requires a failed observation to be recorded. `a_failed_probe_child_stamp_is_recorded_through_the_sweep_refusal` (cross-OS) -> the receipt records `probe_child: {"error": "<cause>"}` and a sweep refusal records `probe_child_at_refusal: "unknown: <cause>"` with the release's record kept. `a_stamped_probe_child_is_recorded_with_its_state` (cross-OS) -> a stamped child serializes as `pid`, `created`, `at_refusal` and is observed again at a sweep. Both observed passing on macOS (provision subset 64 passed); the Windows stamp site compiles through `lint:windows` (exit 0)

## 5. Unchanged guarantees and checks

- [x] 5.1 @equivalence (agent) `mise run //packages/kuru-platform:test` -> exit 0 (lib 24 passed; filesystem 21, local_ipc 4, unix_process_group 2, unix_snapshot 4 passed). `mise run //packages/kuru-memory:test` -> exit 101: lib 593 passed, 9 failed, 6 ignored, 805.85 s, and the integration targets passed (bundle_build 10, memory 5, server_lifecycle 12, supervisor_snapshot 1). All 9 failures were `store::` tests (open_error_reap_tests ×5, open_pool_budget_tests, operational_gc_tests, purge, recovery_tests). Their panics report "No space left on device (os error 28)": the host volume was at 100% with about 7 GiB free. A rerun of exactly those tests (`-- --lib -- store::open_error_reap_tests store::open_pool_budget_tests …`) passed (10 passed, 73.92 s). The `provision::` subset passed alone (62 passed). Observed 2026-10-02
- [x] 5.2 @regression (agent) `mise run format:check` exit 0, `mise run lint` exit 0, root `mise run lint:windows` exit 0 (after one dead-code fix in a test record), `mise run typecheck` exit 0, `mise run docs:check` exit 0 -> observed 2026-10-02
- [~] 5.3 @runtime (agent) native Windows and Linux CI on the PR -> defer: no PR has been opened from this branch yet (the task forbids merging, and no CI run was requested); the Windows rows above wait for it

## Deviations from the design, recorded

- Holder naming (`fs::holders`), C4 and the own-handle guard with its mutation proof are not in this change. This follows the lead ruling's fallback and critique change 1: the instrument cannot see an image section that an exited process object still references. No `windows-sys` feature was added, so the PR-body justification clause does not apply.
- The cospec slug is `provision-leftover-stage-at-cause` (from the task), not the design's `provision-leftover-stage-contract`.
- The sweep also records `inspect` and `open` refusals (a stage that is not a directory, or one that cannot be opened), not only `remove_tree` errors, so `sweep_refusals` counts every attempt that left a stage.
