# Verification

## 1. A new project with a warm template opens with two engine starts [critical]

- [ ] 1.1 @integration (agent) run `warm_template_new_project_uses_two_engine_starts_and_no_migration` against a real Dolt engine with a pre-published template for the process's compiled key -> `engine_ledger` records exactly 2 starts, no new `kuru_migration_*` branch is created, and `Ready` is reported once.
- [ ] 1.2 @integration (agent) run `second_project_reuses_template_without_build` immediately after 1.1 in the same shared cache -> no `.build-*` directory is created for the second project, and its own ledger also shows 2 starts.

## 2. The first project on a fresh machine builds the template once, in three starts [critical]

- [ ] 2.1 @integration (agent) run `first_project_builds_template_once_and_copies_with_three_starts` against a real engine with an empty, private template root -> `engine_ledger` records exactly 3 starts and the schema-migration chain executes exactly once.
- [ ] 2.2 @integration (agent) run `concurrent_new_projects_never_wait_for_a_template_build` with P1 paused inside its build via an injected hook -> P2 (a different project needing the same key) completes on the cold path, with the cold path's own start count, and never observes a `.build-*` or `.stage-*` directory belonging to P1.

## 3. Ineligible and failing templates fall back to the cold path without a caller-visible error [critical]

- [ ] 3.1 @integration (agent) run `template_lock_errors_send_the_opener_cold` with an injected lock-open error, a `TryLockError::Error`, and an identity-verification failure, each on a real engine -> each open completes on the cold path with the cold path's own start count and returns `Ready`, never a template-specific error.
- [ ] 3.2 @integration (agent) run `template_failing_structure_is_quarantined_and_open_goes_cold` with a manifest that fails its structural check -> the open completes on the cold path, and a manifest *read* error (not a verdict) leaves the template in place untouched.
- [ ] 3.3 @integration (agent) run a legacy-import and a configured-`dolt_binary` open against a machine whose cache already holds a published template for the compiled key -> `engine_ledger` shows no `templates/` filesystem access for either open and both take the unchanged cold path.

## 4. Locks are released only after the creation worker's engine is reaped

- [ ] 4.1 @integration (agent) run `cancelled_open_during_template_copy_keeps_startup_lock_until_reap` (open cancelled mid-copy, real engine) -> a second opener of the same project acquires the startup lock only after the ledger records the first worker's engine reap.
- [ ] 4.2 @integration (agent) run the equivalent cancellation test for the build-then-copy path -> the template key's exclusive lock is likewise held until the build engine's reap, and the next exclusive-lock holder sweeps the abandoned build directory rather than waiting on it.

## 5. Unwarmed and build-inside-open fixtures are caught, not masked

- [ ] 5.1 @integration (agent) extend the fixture teardown class-guard test to an unopted-in fixture whose open takes the new build-then-copy path -> the guard fails that fixture, naming the build, exactly as it already does for the existing warm-copy case.
- [ ] 5.2 @unit (agent) run the extended `fresh_open_budget` and `fixture_deadline` unit tests covering the new 2-start and 3-start cases -> both pass with the new budget constants and the existing 4-start cold-path case unchanged.

## 6. No regression to existing-project or cold-path behavior

- [ ] 6.1 @regression (agent) run the full existing `packages/kuru-memory` test suite (`mise run //packages/kuru-memory:test`) after the selector lands -> every existing-project-open and cold-path test (including `temporary_cold()` call sites) passes with its start count unchanged.

## 7. Activity sentence covers the template path [critical]

- [ ] 7.1 @integration (agent) run the new first-launch real-PTY test (task 8.2, private unwarmed cache) -> the terminal frame contains `Creating this project's memory…` while the hold file is present and the build-then-copy path has not yet reached `ready.json`, the sentence is erased by the ready frame, and no other sentence from `memory_activity::SENTENCES` appears before it.
- [ ] 7.2 @integration (agent) re-run `real_pty_accepts_chat_navigation_commands_and_restores_terminal` (its `expect_notice: true` `smoke` call) -> the terminal frame still contains `Creating this project's memory…` across the warm-template copy-and-adoption start (two engine starts via the warmed shared cache), confirming the existing coverage from #148 remains valid against this change's selector.

## 8. Documentation describes the new creation path

- [ ] 8.1 @manual (human) review `docs/memory.md`, `apps/kuru-docs/concepts/memory.md` and the fixture-expectations section of `docs/development.md` against the merged selector behavior -> each accurately states where a new project's data comes from, what the first launch per machine and key pays, and the new 2-start/3-start fixture budgets.
