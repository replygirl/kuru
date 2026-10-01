# Tasks

## 1. Path selector

- [ ] 1.1 Add the creation-path selector in `open_inner` after
      `recover_staging`, choosing cold for a legacy import, a configured
      engine binary, or `Creation::Cold`, and the template path otherwise;
      verify with `template_key_is_not_consulted_for_legacy_or_cold_overrides`
      asserting zero `templates/` filesystem accesses for those three cases.
- [ ] 1.2 Add the shared-lock probe for an already-published template
      (structural check only, no stage yet) and route a structural verdict,
      lock-busy or lock-error result to the cold path with a warning logged,
      never an open error; verify with
      `busy_or_damaged_template_sends_opener_cold_not_error`.
- [ ] 1.3 Add the exclusive-lock probe for "no template published yet" and
      route a lock-busy or lock-error result there to the cold path too;
      verify with the same test extended to the exclusive-lock case.

## 2. Creation worker and locks

- [ ] 2.1 Add the creation worker (`tokio::spawn`, the `run_migration_worker`
      ownership shape) that receives the project's startup lock and the
      template key lock (shared for a copy, exclusive for a build) and
      returns both only after its engine, if any, is reaped; verify with
      `cancelled_open_during_template_copy_keeps_startup_lock_until_reap` and
      an equivalent build-path test.
- [ ] 2.2 Wire `creation_template::copy_into` and the existing adoption
      bootstrap/`adopt_and_mark` into the worker's copy path for an already
      published template; verify with
      `warm_template_new_project_uses_two_engine_starts_and_no_migration`
      (ledger: 2 starts, no new migration branch, one `Ready`).

## 3. First project on a fresh machine

- [ ] 3.1 Wire the exclusive-lock build-then-copy path using
      `creation_template::create_in` and the stage worker's `TemplateBuild`
      job: publish the template, then copy this project's stage from the
      published template, or from the verified build stage if publication
      fails; verify with
      `first_project_builds_template_once_and_copies_with_three_starts`
      (ledger: 3 starts, chain runs once).
- [ ] 3.2 Ensure a second, concurrently opened, different project never waits
      on the first project's build or startup lock and instead takes the
      cold path immediately; verify with
      `concurrent_new_projects_never_wait_for_a_template_build` (P1 paused
      inside its build; P2 completes on the cold path and never observes
      `.build-*` or `.stage-*`).

## 4. Cold fallbacks

- [ ] 4.1 Confirm every cold-fallback condition already specified by the
      template-cache requirement (busy/unreadable key lock, lock
      verification failure, structural verdict, I/O error mid-copy) reaches
      the cold path from the new selector without a caller-visible
      template-specific error; verify with
      `template_lock_errors_send_the_opener_cold` and
      `template_failing_structure_is_quarantined_and_open_goes_cold`.
- [ ] 4.2 Confirm an adoption or shape verdict, or an engine-side failure, on
      the copy's own stage engine returns the existing typed error (no
      in-open retry, no fallback to cold) and leaves the template quarantined
      or untouched per the already-specified rules; verify with
      `adoption_verdicts_are_typed_and_engine_failures_are_not` exercised
      from an ordinary new-project open, not only from template-internal
      tests.

## 5. Test support: budgets and class guards

- [ ] 5.1 Extend `test_support::engine_ledger`-based fresh-open budget
      helpers with the warm-template (2 starts) and build-then-copy (3
      starts) cases, alongside the unchanged cold-path case; verify by
      running the extended `fresh_open_budget` / `fixture_deadline` unit
      tests.
- [ ] 5.2 Extend the fixture teardown class guard so a fixture whose open
      took the build-then-copy path, or quarantined a template, still fails
      unless opted in, covering the new call sites added by this change;
      verify with a test asserting the guard fires for an unopted-in fixture
      that builds inside its own open.
- [ ] 5.3 Add `warm_template_new_project_uses_two_engine_starts_and_no_migration`,
      `first_project_builds_template_once_and_copies_with_three_starts`,
      `second_project_reuses_template_without_build`,
      `concurrent_new_projects_never_wait_for_a_template_build`, and the
      synced-copy ordering test `copied_files_and_directories_are_synced_before_identity`
      as deterministic, real-engine tests; verify by running them locally
      under `mise run //packages/kuru-memory:test`.

## 6. Documentation

- [ ] 6.1 Update `docs/memory.md` with a "New projects" paragraph: template
      origin, what is shared, no secrets in the template, and that the first
      project per machine and key pays the schema-migration chain once;
      verify by `mise run docs:check` (if applicable) or manual review
      against the merged behavior.
- [ ] 6.2 Update `apps/kuru-docs/concepts/memory.md` with the same new-project
      behavior and what a user sees if a template is damaged (silently falls
      back to the cold path); verify with `mise run //apps/kuru-docs:build`.
- [ ] 6.3 Update `docs/development.md`'s fixture-expectations section to
      state the 2-start budget for a fresh warm-template fixture open and the
      3-start budget for the first fixture open on an unwarmed template
      cache; verify by manual review against the updated budget constants
      from task 5.1.

## 7. Verification

- [ ] 7.1 Run the full `packages/kuru-memory` test suite locally
      (`mise run //packages/kuru-memory:test`) and confirm no existing-project
      or cold-path test's start count regressed.
- [ ] 7.2 Run `mise run //packages/kuru-memory:lint` and
      `mise run //packages/kuru-memory:typecheck` and confirm both pass clean
      on the new selector and worker wiring.
- [ ] 7.3 Record which of the above were actually run versus deferred to CI,
      naming the reason for any deferral, before this change is archived.
