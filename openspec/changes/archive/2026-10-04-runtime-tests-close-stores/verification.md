# Verification

## 1. Diagnosis [critical]

- [x] 1.1 @runtime (agent) run 37186425537 job 111389939936 log and `ci-coverage-diagnostics-ubuntu-latest-partition-6-attempt-1` -> observed: 283 tests from 75 executables passed. The last executable was `kuru_runtime-c677c8631f83e0f5` (34 tests, 07:52:05.75-07:52:24.91), and the last to finish were `tests::manual_focus_and_relationship_validation_reject_invalid_topology`, `tests::mutation_and_opaque_calls_split_native_read_waves` and `tests::relationships_preserve_their_own_history_without_access_to_part_notes`. The error came at 07:52:32.62: "new kuru-12898-10634519169386027498_2.profraw". The artifact (`failure.txt`, `inventory.json`, `runner-ledger.jsonl`, stdout logs) has no spawn rows, because `cbebf7b7` predates #208, and nothing else names pid 12898. So the writer is inferred, not measured, to be a kuru-memory Dolt supervisor of a runtime test that dropped its store; at `cbebf7b7` no runtime test closed its stores. The signature differs from #208's `…682` because `%m` is per binary build, not per executable name. `cbebf7b7` includes #211's kuru-memory change and #133's PR build (5030b4bc) does not, while Cargo's `c677c8631f83e0f5` metadata hash stays the same across both.
- [x] 1.2 @equivalence (agent) whole `kuru_runtime` lib on `94cf53c5`, uninstrumented, `KURU_TEST_DOLT_LOG_DIR` set -> observed: 236 passed (exit 0); 300 supervisor spawns, 300 `owner_finished`, 0 `owner_dropped_live`. #208's success-path teardown already covers every test, `dream::cancellation_tests` included, so this change's scope is the panic/early-return paths and enforcement.

## 2. Every path closes [critical]

- [x] 2.1 @regression (agent) `tests::dreaming_rejects_last_role_removal_unknown_roles_names_and_capacity_overflow` with an injected env-gated failing assertion before its teardown (not committed), instrumented (`-C instrument-coverage`, a separate target dir that was deleted afterwards), with a runner-like harness (own process group, group SIGKILL after the root exits, then a 30 s watch) -> observed: on `94cf53c5`, 2 of 9 runs left a late `kuru-<pid>-1775106161973429056_0.profraw` (the local kuru-memory supervisor signature; the test executable's is `13325969203012773943`). With the lifecycle trace, 3/3 runs recorded `owner_dropped_live` and no `owner_finished`. On the fix, 0 of 9 runs were late, and 3/3 traced runs recorded `owner_finished` and 0 `owner_dropped_live`. Uninjected controls: 0 late both before and after.
- [x] 2.2 @unit (agent) `cargo test -p kuru-memory --lib --all-features -- test_support::closing` ×3 -> observed: 7 passed each run. They cover a dropped store closed and awaited; a panic, with the store closed and the original panic resumed; an `Err` early return closed; a second close; a supervisor outside the scope failing "test … has not been reaped" by name; scopes that do not nest and end when dropped; and five sequential opens in one scope.
- [x] 2.3 @regression (agent) fixture-permit regression: the first implementation retained `MemoryStore` clones -> observed: the full runtime run hung, with `hook_tests::speaker_hook_cannot_substitute…` and `speaker_observe_receives…` stuck at their 4th and 2nd store opens. The trace showed 3 and 1 spawns. The clones held the process's 4 fixture permits. Retaining only the `Server` fixed it, and `closed_stores_in_a_scope_release_their_fixture_permits` covers it.
- [x] 2.4 @regression (agent) source-scan guard `every_async_test_runs_its_body_in_the_closing_scope` -> observed: before its fixture was assembled at runtime, the guard failed by name on its own unwrapped fixture text ("tests.rs:298: drops_its_store"). `the_closing_scan_names_a_test_outside_the_scope` asserts `["10: drops_its_store"]`. It passes on the tree (238 tests; the guard requires that it found at least one async test). The scan later moved to `kuru_memory::test_support` with its fixture test (see 3.4), so the runtime lib has 237.
- [x] 2.5 @regression (agent) a trailing teardown that is load-bearing -> observed: `preferences_survive_reopening…` failed its fixture root's quiescence check ("in-process Dolt supervisor … has not been reaped") after its trailing `close_stores` was removed. The scope closes only after the body's locals drop, and its `kuru_memory::test_support::tempdir()` root drops inside the body. The close was restored with a comment. It is the only removed teardown in a test with a fixture root, found by script.

## 3. No dropped store in the suite

- [x] 3.1 @regression (agent) `KURU_TEST_DOLT_LOG_DIR=… mise run //packages/kuru-runtime:test` ×3 on the fix -> observed: exit 0 each run, 238 passed (137.9 s, 134.6 s, 137.6 s); 298 spawns, 298 `owner_finished`, 0 `owner_dropped_live` each run. Per-label spawn counts equal 1.2's, except that `accounting_tests::admitted_without_observation…` has 1 rather than 3. The baseline's 2 extra spawns were a template build (a `.staging-…` open, then its activation) under the first test's label, made after the shared cache's `mbx gc`. No test opens fewer stores. The baseline's longer wall time (310 s) is not attributed.
- [x] 3.2 @regression (agent) whole `apps/kuru-tui` suite, `KURU_TEST_DOLT_LOG_DIR=… mise run //apps/kuru-tui:test`, before and after the kuru-tui wrap -> observed: before, exit 0, 288 passed, 4 ignored; 326 spawns, 311 `owner_finished`, 15 `owner_dropped_live` from 13 tests: `ui_runtime::runtime_adapter_projects_real_relationship_completion_and_sanitized_route`, `preferences::invocation_overrides_resume_and_other_projects_do_not_replace_saved_selections`, `preferences::an_explicit_framework_override_is_validated_against_its_own_part_budget` and ten `ui::runtime_tests` (`real_loop_eof_read_and_draw_failures…` 3, the others 1 each, including the two notice tests that dropped their store before reopening it). `cli`, `embedded_runtime`, `terminal` and `trust` recorded none. After, exit 0, 289 passed (the new guard), 4 ignored; 328 spawns, 328 `owner_finished`, 0 `owner_dropped_live`. Per-label spawn counts are equal except `memory_notice::tests::failed_headless_write_or_flush…`, 4 rather than 2: a template build (a `.staging-…` open, then its activation) under that label, as in 3.1.
- [x] 3.3 @regression (agent) the 18 kuru-tui tests in `ui::runtime_tests`, `memory_notice`, `ui_runtime` and `preferences`, plus the guard, `cargo test -p kuru --all-features --locked --lib --test ui_runtime --test preferences` with `RUST_TEST_THREADS=2`, `KURU_TEST_SUPERVISOR_PREPARED=1` and the lifecycle trace, ×3 -> observed: exit 0 each run, 19 passed; 37 spawns, 37 `owner_finished`, 0 `owner_dropped_live` each run.
- [x] 3.4 @unit (agent) shared scan: `cargo test -p kuru-memory --lib --all-features -- test_support::closing` -> observed: 8 passed, including `the_closing_scan_names_a_test_outside_the_scope` (moved from the runtime, asserting `["10: drops_its_store"]`). `mise run //packages/kuru-runtime:test` -> exit 0, 237 passed, with `every_async_test_runs_its_body_in_the_closing_scope` on the shared scan. `tests::every_store_opening_async_test_runs_its_body_in_the_closing_scope` passes in kuru-tui (3.2). With one `tests/cli.rs` body temporarily taken out of the scope (not committed), it failed naming `"tests/cli.rs:123: cli_post_turn_failure_reports_separately_after_completed_json_answer"`.

## 4. Static and package checks

- [x] 4.1 @unit (agent) `mise run format:check`, `typecheck`, `lint`, `lint:windows` -> observed: each exit 0. `lint:windows` checks the wrapped `cfg(windows)` `windows_tool_tests`. Re-run after the kuru-tui change: each exit 0, with `lint:windows` also checking kuru-tui's wrapped `cfg(windows)` tests.
- [x] 4.2 @unit (agent) `mise run cospec -- validate --all --strict` -> observed: exit 0
- [~] 4.3 @runtime (agent) a native CI coverage shard with this change -> defer: CI reruns are not allowed for this task; the PR's own CI runs every partition

## Integrated with current main (2026-10-04)

Rebased all seven reviewed commits onto main
`53ce62a2d130b92730a48ad9a3816bea04e42261`, including PR #223's foreign-directory
startup-probe correction and PR #218's event-based dream wait. The only conflict
was `accounting_tests.rs`: retained the complete main dream-test body inside the
closing scope, including progress rearming, early-finish/stall diagnostics,
cancellation settlement and explicit close-before-reopen boundaries. Six commits
kept equivalent patches. Across the previously reviewed runtime/TUI and memory
closing-support sources, only that accounting file differs from the old head;
its 58-line integration diff contains the expected PR #218 changes.

`mise run format:rust:fix` completed successfully. The package-owned focused test
`abandoned_dream_keeps_usage_after_reopen_without_advancing_main` passed 1/1 in
3.54 seconds, using the checksum-pinned local bundle in offline mode and a
command-local soft descriptor limit of 4096. This is local macOS evidence, not a
claim about hosted limits or native Windows/Linux execution. Full final-head CI
remains required; the old macOS foreign-probe failure was not manually rerun.
