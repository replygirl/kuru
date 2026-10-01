# Verification

## 1. A new project with a warm template opens with two engine starts [critical]

- [x] 1.1 @integration (agent) run `warm_template_new_project_uses_two_engine_starts_and_no_migration` against a real Dolt engine with the warmed shared template -> `engine_ledger` records exactly 2 starts under the fixture, the store's histories equal the template's plus one adoption commit per ref (no migration ran), no template-era revision is pooled, `Ready` is reported once and the shared template is unchanged; observed 2026-10-01 on macOS: passed
- [x] 1.2 @integration (agent) run `second_project_reuses_template_without_build` against a private template root the first project built -> the second project records 2 starts, no further build start under the template root, and the template root's entries and published identity are unchanged; observed 2026-10-01 on macOS: passed

## 2. The first project on a fresh machine builds the template once, in three starts [critical]

- [x] 2.1 @integration (agent) run `first_project_builds_template_once_and_copies_with_three_starts` with an empty, private template root -> `engine_ledger` records exactly 3 starts, exactly 1 of them under the template root (one build engine, chain once), the store is template-born, and the root holds only the published template and its lock file; observed 2026-10-01 on macOS: passed
- [x] 2.2 @integration (agent) run `concurrent_new_projects_never_wait_for_a_template_build` with P1 paused inside its build engine -> P2 (a different project, same key) completes on the cold path with 4 starts and cold histories while P1 is still paused, leaves the template root's entries exactly as it found them (P1's `.build-*` included) and preserves no stage; P1 then publishes and completes with 3 starts; observed 2026-10-01 on macOS: passed

## 3. Ineligible and failing templates fall back to the cold path without a caller-visible error [critical]

- [x] 3.1 @integration (agent) run `template_lock_errors_and_busy_locks_send_the_opener_cold` with an injected lock-file open error, `TryLockError::Error`, lock verification failure, manifest read error, and a key lock held exclusively by another holder -> each open succeeds on the cold path with 4 starts and cold histories, the published template is unchanged and nothing is quarantined or preserved; observed 2026-10-01 on macOS: passed
- [x] 3.2 @integration (agent) run `damaged_templates_send_the_opener_cold_and_preserve_copy_remnants` with a structural verdict, a digest mismatch mid-copy and an injected read error mid-copy -> each open succeeds cold with 4 starts; the structural verdict quarantines before any copy and preserves nothing; the digest mismatch preserves one unstarted copy remnant (no identity, 0 starts) and quarantines; the read error preserves the remnant and leaves the template published; observed 2026-10-01 on macOS: passed
- [x] 3.3 @integration (agent) run `legacy_import_and_configured_binary_take_cold_path` -> a configured `dolt_binary` and a legacy import each open cold with 4 starts, the import is recorded in the activation record, and the private template root is never created; observed 2026-10-01 on macOS: passed

## 4. Locks are released only after the creation worker's engines are reaped [critical]

- [x] 4.1 @integration (agent) run `cancelled_open_during_template_copy_keeps_startup_lock_until_reap` (opener cancelled at the copy's ready-marker boundary) -> a second opener acquires the startup lock only when the ledger shows no live engine under the fixture, and the next open preserves the unready stage; observed 2026-10-01 on macOS: passed
- [x] 4.2 @integration (agent) run `cancelled_open_during_template_build_finishes_and_leaves_a_ready_stage` (first project's opener cancelled while its worker is paused inside the build engine) -> the startup lock is held while paused; after resuming, the worker publishes the template and leaves one ready stage, a second opener acquires the startup lock only after every engine is reaped, and the next open reuses the ready stage (template-born, nothing preserved); observed 2026-10-01 on macOS: passed
- [x] 4.3 @integration (agent) run the existing `cancelled_build_releases_key_lock_only_after_reap` (#147, the build called directly) -> the key's exclusive lock is acquirable only once the build engine was reaped; observed 2026-10-01 on macOS: passed

## 5. Verdicts on the copy's own engine and engine failures

- [x] 5.1 @integration (agent) run `shape_verdict_on_the_copy_fails_the_open_and_quarantines_the_template` with a published template holding an extra branch -> the open fails with the typed `TemplateVerdict` ("unexpected branch") after 1 start, no active directory, the unready template stage preserved, the judged template quarantined; the next new project rebuilds and copies (2 starts in its own directory); observed 2026-10-01 on macOS: passed
- [x] 5.2 @integration (agent) run `engine_failure_on_the_copy_preserves_the_stage_and_keeps_the_template` (ready-marker observer closed before release) -> the open fails without a verdict, the unready stage is preserved, the template is published unchanged and nothing quarantined; the next new project copies with 2 starts; observed 2026-10-01 on macOS: passed

## 6. Fixture class guards are live

- [x] 6.1 @integration (agent) run `fixture_open_that_builds_the_shared_template_fails_teardown` (child process with a private shared cache, options claiming a warm-up) -> the child's open builds the shared template in 3 starts (2 in the project, 1 build) and its fixture root's teardown fails naming the build; observed 2026-10-01 on macOS: passed
- [x] 6.2 @integration (agent) run `test_template_build_copies_the_store_template_with_two_starts` -> every new test-template source store opens with 2 starts and the shared store template is unchanged; observed 2026-10-01 on macOS: passed
- [x] 6.3 @unit (agent) run `fresh_open_budgets_follow_each_creation_path` and `single_stall_defaults_match_the_reviewed_bounds` -> (starts, closes) are (2,1), (3,2), (4,3); each path's budget adds one start and close; the default fresh-open budget and `fixture_deadline` keep the cold four-start values; observed 2026-10-01 on macOS: passed

## 7. Activity sentence covers the template path [critical]

- [x] 7.1 @e2e (agent) run `real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds` (real PTY, private empty cache, owner held at `CreatingDatabase`) -> after release, while the cache's `templates/` lists a `.build-*` entry, the screen shows `Creating this project's memory…` and no other sentence was written after it; at the completed composer frame the startup bytes hold no later sentence, the line is erased, one template is published and no transient entry remains; observed 2026-10-01 on macOS: passed
- [x] 7.2 @e2e (agent) run `real_pty_accepts_chat_navigation_commands_and_restores_terminal` with `smoke` extended -> on the first run (warmed shared cache), while the project's `*.staging-*` copy exists the screen shows `Creating this project's memory…` and no later sentence replaces it before the erase; observed 2026-10-01 on macOS: passed
- [x] 7.3 @e2e (agent) mutation check: report `WaitingForProjectOwnership` in place of `OpeningDatabase` before the creation worker, then run 7.1 -> the test fails naming the waiting sentence as having replaced the creating sentence; observed 2026-10-01 on macOS: failed as expected ("Waiting for another copy of Kuru to finish with this project's memory…" replaced the creating sentence), mutation reverted
- [x] 7.4 @integration (agent) run `cli_new_project_shows_engine_preparation_then_creation_and_keeps_json_on_stdout` (empty cache, now building the template) -> the last sentence before ready is still `Creating this project's memory…`; observed 2026-10-01 on macOS: passed

## 8. No regression to existing-project, cold-path or spawned-binary fixtures

- [x] 8.1 @regression (agent) run `mise run //packages/kuru-memory:test` -> every test passes; observed 2026-10-01 on macOS: exit 0, lib 515 passed, 0 failed, 4 ignored
- [x] 8.2 @regression (agent) run `mise run //apps/kuru-tui:test` and `mise run //packages/kuru-runtime:test` -> every test passes, including the fresh-cache fixtures whose depth budgets now cover the store template; observed 2026-10-01 on macOS: both exit 0 (kuru-runtime 220 passed; kuru-tui every target passed after the trust fixture depth fix, including the packaged `packaged_install_and_update_preserve_complete_offline_memory`)

## 9. Documentation describes the new creation path

- [~] 9.1 @manual (human) review `docs/memory.md`, `apps/kuru-docs/concepts/memory.md` and the fixture section of `docs/development.md` expecting: each states where a new project's data comes from, what the first launch per machine and key pays, the sentence a user sees, what a damaged template does, and the fixture start counts. -> defer: the maintainer reviews the docs in the pull request

## 10. Open-time harness (release build)

- [x] 10.1 @benchmark (agent) run the `ci/open-time-report` harness N=10 against release builds of the base and head -> engine starts per case go from 4/1/0/4 to 3/1/0/2 (first-launch, cold-existing, warm-reopen, new-project); observed 2026-10-01 on macOS (1-min load below 14 throughout): base 1c93476f 4/1/1/4, head 3587b4c1 3/1/1/2 (warm-reopen starts one engine in both because the owner now retires immediately); open p50 first-launch 5615 -> 6188 ms, cold-existing 536 -> 536 ms, new-project 2507 -> 1262 ms

## 11. Review fixes (pull request #151) [critical]

- [x] 11.1 @integration (agent) run `failed_template_build_fails_the_open_without_a_cold_retry` (empty private template root; the build's own shape check refuses an extra branch) -> the open fails with "build the memory store template" and the refusal, after exactly 1 engine start (the build engine) and none in the project; nothing published or quarantined, no active directory, no stage left; observed 2026-10-01 on macOS: passed
- [x] 11.2 @integration (agent) run `failed_stage_start_leaves_the_copy_for_the_next_open_to_preserve` (stage identity names another build's key) -> ordinary error naming both keys, template unchanged, nothing preserved yet, one unready stage left in place; the next open preserves it under `interrupted/` without a start and makes 2 starts copying the template; observed 2026-10-01 on macOS: passed
- [x] 11.3 @integration (agent) run `adoption_verdict_quarantines_and_leaves_the_stage_in_place` (published template with a foreign usage placeholder row) -> typed `TemplateVerdict` "is not the compiled template placeholder", the judged template quarantined, one unready stage left in place, nothing preserved yet; observed 2026-10-01 on macOS: passed
- [x] 11.4 @integration (agent) run `fixture_open_that_quarantines_the_shared_template_fails_teardown` (child process; its shared cache holds a template with a damaged manifest) -> the child's open is cold with 4 starts, quarantines the shared template and its fixture teardown fails naming the quarantine; observed 2026-10-01 on macOS: passed
- [x] 11.5 @integration (agent) run `first_project_reports_creation_before_its_template_build` -> while the build is paused, `CreatingDatabase` has been reported; only `OpeningDatabase` and `Ready` follow it; observed 2026-10-01 on macOS: passed
- [x] 11.6 @integration (agent) run `copied_files_and_directories_are_synced_before_identity` with paths compared within the stage -> passed on macOS 2026-10-01; Windows is proven only by CI on the pushed head
- [x] 11.7 @regression (agent) run `//packages/kuru-memory:test` filtered to `store::creation_template`, `store::template_stage_tests`, `store::stage_worker` and `store::tests::observed` -> 66 passed, 0 failed; observed 2026-10-01 on macOS with a private `KURU_DOLT_CACHE` (the shared test cache's engine directory was found incomplete, modified by another process; it was left untouched). Earlier in the same session `cancelled_open_during_template_build_finishes_and_leaves_a_ready_stage` failed twice in that filtered set by waiting out its bound for the build's pause, and passed alone twice and in the later runs; the test now reports how the open ended instead of waiting out its bound. The full package, kuru-tui and coverage runs are left to CI
