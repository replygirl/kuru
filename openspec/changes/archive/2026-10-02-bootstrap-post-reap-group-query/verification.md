# Verification

Local evidence: macOS 27.0 arm64 (this worktree, branch
`fix/bootstrap-post-reap-group-query`, 2026-10-02). CI evidence is not yet
available and is not claimed.

## 1. A reaped group recycled by another user is not a cleanup failure [critical]

- [x] 1.1 @regression (agent) `bootstrap_install::post_reap_query_accepts_a_group_recycled_by_another_user` queries a real process group led by another user's process (signal zero refused with `EPERM`) through the bootstrap cleanup query -> observed: `ok` in `mise run //packages/kuru-delivery:test` (packages/kuru-delivery/tests/bootstrap_install.rs:941, query at tests/support/bootstrap_process.rs:262). The fixture group is selected only when the pre-fix query `test_kill_process_group` returns `EPERM` for it (bootstrap_install.rs:932), the exact errno the pre-fix cleanup reported as `post-reap process group query: Operation not permitted`, so the pre-fix code fails this case by construction
- [x] 1.2 @integration (agent) `unix_snapshot::foreign_group_is_classified_as_recycled_without_error` -> observed: `ok` in `mise run //packages/kuru-platform:test` (packages/kuru-platform/tests/unix_snapshot.rs:250)
- [x] 1.3 @unit (agent) `unix::tests::owned_listing_resolves_permission_only_with_foreign_evidence` and `unix::tests::group_classification_requires_evidence_before_ignoring_permission` -> observed: `ok`; empty listing `Absent`, foreign-only `Recycled`, own member (effective, real, zombie) or unavailable listing `PermissionDenied` (packages/kuru-platform/src/unix.rs:774, listing decision at unix.rs:656)

## 2. A live member of ours is never dismissed [critical]

- [x] 2.1 @integration (agent) `bootstrap_install::post_reap_query_rejects_a_live_group_of_ours_with_its_listing` and `unix_snapshot::live_group_of_ours_is_classified_as_a_survivor_with_its_listing` -> observed: both `ok`; failure text carries `pid=<root> ` (bootstrap_install.rs:953, unix_snapshot.rs:271)

## 3. Cleanup bounds do not grow

- [x] 3.1 @unit (agent) `command::bounded_unix::tests::slow_listing_runs_once_and_keeps_the_cleanup_deadline` (persistent `EPERM`, listing sleeps its full budget) -> observed: `ok`; exactly one listing, `TimedOut`, elapsed asserted below `CLEANUP_TIMEOUT` + 500 ms (packages/kuru-delivery/src/command.rs:556); `persistent_permission_remains_a_bounded_error` asserts one listing per cleanup (command.rs:525)
- [x] 3.2 @unit (agent) `unix::tests::permission_listing_lists_once_within_the_cleanup_budget`, `permission_listing_skips_spent_budgets_and_invalid_groups`, `permission_listing_runs_off_the_executor_and_reports_task_failure`, `owned_listing_uses_the_real_listing_for_its_own_group` -> observed: all `ok` (unix.rs:813, 870, 908, 946); `presence_after_reap` is again one signal-zero query (unix.rs:241)

## 4. Test identity uses recorded evidence

- [x] 4.1 @integration (agent) `bootstrap_install` producer tests (`manifest_and_archive_downloads_are_bounded_including_a_producer_that_stays_open`, `sigterm_during_download_reaps_both_children_and_preserves_the_previous_binary`) -> observed: both `ok` (bootstrap_install.rs:1157, 1268), using the `exec -a "$FIXTURE_ROOT/producer"` tag (verified to appear in `ps -o args=` with macOS `/bin/bash` 3.2.57) and recorded group rows
- [x] 4.2 @integration (agent) `review_tests::cancelled_shell_turn_reaps_the_observed_owned_process_without_replay` and `hooks::tests::cancellation_awaits_reaping_the_started_owned_hook_tree` -> observed: `ok` in `mise run //packages/kuru-runtime:test -- cancelled_shell_turn` and `mise run //packages/kuru-connectors:test` (packages/kuru-runtime/src/review_tests.rs:771, packages/kuru-connectors/src/hooks.rs:1765)

## 5. Static and package checks

- [x] 5.1 @integration (agent) `mise run //packages/kuru-platform:test` -> observed: exit 0, no failures
- [x] 5.2 @integration (agent) `mise run //packages/kuru-delivery:test` -> observed: exit 0, no failures (lib 195 passed)
- [x] 5.3 @integration (agent) `mise run //packages/kuru-connectors:test` (297 passed, exit 0) and `mise run //packages/kuru-runtime:test -- cancelled_shell_turn` (1 passed, exit 0) -> observed: pass; the full runtime suite was not run because only this test changed there
- [x] 5.4 @integration (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run lint:tooling` -> observed: each exit 0; hk pre-commit format, tooling and conventional steps passed on each commit
- [~] 5.5 @runtime (agent) CI macOS coverage partition on the PR -> defer: no PR or CI run exists for this branch yet; it must pass before merge
