# Verification

Windows behaviour cannot run on this macOS host. Windows-only tests are
compiled and linted through root `mise run lint:windows`, which runs clippy
with `--all-targets` for `x86_64-pc-windows-msvc` in every package that owns
the task. Their native result is CI's. Rows that state a deterministic red
before the fix fail on every run without the fix. Rows marked
"fails-without-fix only when a stage remains" depend on a native hold that
the host does not control.

## 1. The concurrent cold-provision test asserts the contract [critical]

- [ ] 1.1 @regression (agent) `concurrent_cold_windows_provision_publishes_one_verified_native_identity` on windows-latest coverage -> passes whether or not the winner's stage is retained. A retained stage is accepted only when it is receipted, `published: true`, its first cause matches a retried-class row with `attempts` present, it records `sweep_refusals >= 1`, and no sweep-skipped record was observed. Every receipt is written to stderr. Fails without the fix only when a stage remains (the original failure, run 36930175874 / job 110597536882)
- [ ] 1.2 @unit (agent) cause-table classification of the receipt `first_cause` strings (cross-OS unit test over representative strings for every row and two non-retried causes) -> every retried row matches its name, and a non-retried cause or a missing `attempts` is refused

## 2. Sweep refusals are recorded and `.leftovers` follows its receipts [critical]

- [ ] 2.1 @regression (agent) `sweep_refusal_rewrites_only_the_refusal_fields` (every OS; Unix refuses through a nested symlink, Windows through a nested file held without `FILE_SHARE_DELETE`) -> two sweeps leave the stage (same identity) and its receipt, keep every original field byte-equal, record `sweep_refusals` 1 then 2 with `last_sweep_refusal` naming the phase, the cause, the descendant `sub/<blocker>` and (on Windows) `os_error` 32, and stay under `LEFTOVER_RECEIPT_LIMIT`. After the blocker is released, the next sweep collects the stage and its receipt and removes `.leftovers`. Deterministically red before the fix on macOS
- [ ] 2.2 @regression (agent) `sweep_that_collects_the_last_receipt_removes_leftovers` -> `.leftovers` is gone after the last receipt is collected. Deterministically red before the fix
- [ ] 2.3 @unit (agent) `sweep_keeps_leftovers_while_a_receipt_remains` and `sweep_keeps_leftovers_whose_staging_holds_a_record` -> `.leftovers` and its non-empty `staging` are kept
- [ ] 2.4 @regression (agent) `a_refused_leftovers_removal_is_reported` (Unix: a read-only version directory; Windows: `staging` held without `FILE_SHARE_DELETE`) -> `.leftovers` stays and exactly one `kuru.memory` "kept" record names it with its cause. Deterministically red before the fix on macOS
- [ ] 2.5 @unit (agent) the five direct-sweep tests moved from the Unix-only module (`sweep_collects_only_receipted_stages`, `collect_leftover_receipt_never_counts_a_receipt_it_could_not_remove`, `sweep_leaves_a_stage_and_its_receipt_when_removal_is_rejected`, `sweep_reports_the_cap_without_deleting_any_uncollectable_stage`, `sweep_leaves_an_oversized_or_unparsable_receipt_as_remaining`) -> pass on macOS and compile for Windows

## 3. Every lock acquisition sweeps first; warm opens never wait

- [ ] 3.1 @unit (agent) `warm_sweep_collects_a_receipt_when_the_lock_is_free` and `warm_sweep_leaves_a_receipt_untouched_when_the_lock_is_busy` (every OS; `warm_sweep_if_receipted` called directly) -> a free lock collects the stage and receipt and removes `.leftovers`. A test-held lock leaves both byte-identical with no `sweep_refusals`, and the next acquisition collects them
- [ ] 3.2 @regression (agent) `warm_sweep_reports_a_failed_lock_attempt` (`.install.lock` replaced by a directory) -> exactly one "leftover install stage sweep skipped" record for this cache, and nothing on disk changes. Deterministically red before the fix
- [ ] 3.3 @unit (agent) the Unix `cold_provision_sweeps_a_receipted_stage_before_creating_a_new_one`, `warm_open_skips_the_sweep_when_the_installation_lock_is_busy` and `exhausted_stage_cleanup_publishes_the_engine_and_receipts_the_retained_stage`, extended -> the cold and warm paths leave no `.leftovers` after collecting, and a busy warm open records no refusal

## 4. Refusal evidence: descendant and probe child

- [ ] 4.1 @regression (agent) kuru-platform `remove_tree_names_the_refusing_descendant` (Unix) -> the `RemovalError` for a nested symlink names `sub/link` relative to the root, and the root failure names none. Red before the fix (with `descendant` always `None`)
- [ ] 4.2 @unit (agent) kuru-platform Windows `remove_tree_names_a_held_nested_descendant` -> a nested file held without `FILE_SHARE_DELETE` is named `sub/held`, with native error 32. Compiled through `lint:windows`; native result is CI's
- [ ] 4.3 @unit (agent) kuru-platform Windows `process_object_retained_matches_only_the_exact_identity` -> the current process and a child whose duplicated handle is held are `true`, and the same id with a different creation time is `false`. No control asserts `false` after a drop, because another process may legitimately hold the object. Compiled through `lint:windows`; native result is CI's
- [ ] 4.4 @unit (agent) `a_stage_cleanup_report_names_the_refusing_descendant` -> a `RemovalError` in the cause chain carries its descendant into the report and the receipt
- [ ] 4.5 @integration (agent) Windows `forced_published_cleanup_failure_records_the_probe_child` (real embedded engine, forced cleanup failure) -> the receipt records a probe child with non-zero `pid` and `created` and an `at_refusal` of `retained`, `released` or `unknown: …`. The warm open then either collects the stage, leaving no receipt, or records a sweep refusal. Compiled through `lint:windows`; native result is CI's
- [ ] 4.6 @unit (agent) Windows `cancelled_activation_recovery_receipts_a_held_stage_before_releasing_the_lock` -> its stage is now collected through the product sweep (`collected == 1`, `.leftovers` gone) after the holder drops, in place of `remove_dir_all`. Compiled through `lint:windows`; native result is CI's

## 5. Unchanged guarantees and checks

- [ ] 5.1 @equivalence (agent) `mise run //packages/kuru-memory:test` and `mise run //packages/kuru-platform:test` on macOS -> every existing test passes
- [ ] 5.2 @regression (agent) `mise run format:check`, `mise run lint`, root `mise run lint:windows`, `mise run typecheck`, `mise run docs:check` -> clean exits
- [ ] 5.3 @runtime (agent) native Windows and Linux CI on the PR -> the Windows rows above pass natively
