# Tasks

## 1. Path selector

- [x] 1.1 Add the creation-path selector (`creation_worker::select`) in
      `open_inner` after `recover_staging`, choosing cold for a legacy import,
      a configured engine binary, or `Creation::Cold`, and the template path
      otherwise; verify with `legacy_import_and_configured_binary_take_cold_path`
      (four starts each, the private template root never created) and the
      `Creation::Cold` fixtures (`template_born_store_matches_cold_store`, the
      cold-path cancellation and pool-budget tests).
- [x] 1.2 Route a busy key lock, a lock-file open, lock or verification error
      and a manifest read error to the cold path with a warning, never an
      open error, and never touching the template; verify with
      `template_lock_errors_and_busy_locks_send_the_opener_cold`.
- [x] 1.3 Route a structural verdict (quarantine first) and a verdict or I/O
      error mid-copy (copy remnant preserved without an engine start; only a
      verdict quarantines) to the cold path in a new stage; verify with
      `damaged_templates_send_the_opener_cold_and_preserve_copy_remnants`.

## 2. Creation worker and locks

- [x] 2.1 Add the creation worker (`creation_worker::run`, `tokio::spawn`, the
      `run_migration_worker` ownership shape) that owns the startup lock and
      returns it only after its engines are reaped, and run each copy on a
      blocking thread that holds the key lock; verify with
      `cancelled_open_during_template_copy_keeps_startup_lock_until_reap`,
      `cancelled_open_during_template_build_finishes_and_leaves_a_ready_stage`
      (an opener cancelled inside the build: the worker keeps the startup
      lock, finishes, and the next open reuses its ready stage) and, for the
      build's key lock, the existing
      `cancelled_build_releases_key_lock_only_after_reap`.
- [x] 2.2 Wire `creation_template::create_in`, the stage identity record
      (written last) and `StageWorker::adopt_and_mark` into the worker's copy
      path; verify with
      `warm_template_new_project_uses_two_engine_starts_and_no_migration`
      (ledger: 2 starts, histories of the template plus one adoption commit
      per ref, `Ready` once, no template-era pool) and
      `copied_files_and_directories_are_synced_before_identity`.

## 3. First project on a fresh machine

- [x] 3.1 Build-then-copy through `create_in` under the exclusive key lock;
      verify with `first_project_builds_template_once_and_copies_with_three_starts`
      and `second_project_reuses_template_without_build`.
- [x] 3.2 A concurrent different project never waits on the build; verify
      with `concurrent_new_projects_never_wait_for_a_template_build` (first
      project paused inside its build engine; the second completes cold with
      four starts and leaves the template root unchanged; the first then
      completes with three).

## 4. Verdicts and engine failures on the copy

- [x] 4.1 A verdict on the copy's own engine fails the open with the typed
      verdict, no retry, preserves the stage and quarantines the judged
      template (`creation_template::quarantine_after_adoption`); verify with
      `shape_verdict_on_the_copy_fails_the_open_and_quarantines_the_template`.
- [x] 4.2 A non-verdict failure on the copy's engine fails the open and leaves
      the template untouched; verify with
      `engine_failure_on_the_copy_preserves_the_stage_and_keeps_the_template`.

## 5. Test support: budgets and class guards

- [x] 5.1 Add `FreshOpen::{Template, FirstProject, Cold}` and
      `fresh_open_budget_of`, keeping the cold case as the default fresh-open
      budget and in `fixture_deadline`; verify with
      `fresh_open_budgets_follow_each_creation_path` and
      `single_stall_defaults_match_the_reviewed_bounds`.
- [x] 5.2 Make the fixture class guard live: verify the real trigger with
      `fixture_open_that_builds_the_shared_template_fails_teardown` (child
      process with a private cache; the open builds the shared template in
      three starts and teardown fails naming the build).
- [x] 5.3 Add `ordinary_open_never_pools_a_pre_adoption_revision` (T9) and
      `test_template_build_copies_the_store_template_with_two_starts` (T24);
      keep cold-path fixtures that pause or delay a cold staging job on
      `Creation::Cold`; adjust
      `copy_remnant_without_identity_is_preserved_without_engine_start` to the
      template-born store the open now creates.
- [x] 5.4 Remove the `dead_code` expectations that held while no open used the
      template, keeping targeted ones for warm-up-only items.

## 6. Documentation

- [x] 6.1 Update `docs/memory.md`: new projects come from the template, start
      counts per case, when a project is built directly, and what a user sees
      when a template is damaged.
- [x] 6.2 Update `apps/kuru-docs/concepts/memory.md` (what the first launch
      pays, what a damaged template does) and the `cache_dir` notes in
      `docs/configuration.md` and `apps/kuru-docs/reference/configuration.md`.
- [x] 6.3 Update `docs/development.md` fixture expectations: two starts for a
      fresh fixture open, `FreshOpen`, `Creation::Cold` for paused cold jobs,
      private template roots.
- [x] 6.4 Spec delta: the creation path requirement, and the byte-scan
      correction in "Per-machine store template cache".

## 7. Verification

- [x] 7.1 Run the full `packages/kuru-memory`, `packages/kuru-runtime` and
      `apps/kuru-tui` test tasks locally.
- [x] 7.2 Run `format:check`, kuru-memory `lint`, `lint:windows` and
      `typecheck`, and `docs:check`.
- [x] 7.3 Record which checks ran and which are deferred to CI, with reasons.

## 8. Activity sentence covers the template path

- [x] 8.1 Confirm (reviewed, not re-implemented) that `open_inner` reports
      `MemoryOpenStage::CreatingDatabase` before `creation_worker::select` and
      before the worker's own `OpeningDatabase` report, so #148's stage
      mapping (D3: R5 shows S3 on `CreatingDatabase`, R7 keeps it on
      `OpeningDatabase`) covers the build-then-copy and warm-copy paths with
      no code change and no new sentence or stage; see design.md's "whole
      template path" decision for the exact line references.
- [x] 8.2 Add one real-PTY test in `apps/kuru-tui/tests/terminal.rs` for a
      first launch against a private, empty cache
      (`real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds`):
      hold the open at `CreatingDatabase`, wait for the complete `CREATING`
      bytes, release, wait until the cache's `templates/` lists a `.build-*`
      entry, and assert the screen shows `CREATING` with no later sentence;
      at the completed composer frame assert no later sentence, the erase,
      one published template and no transient entry.
- [x] 8.3 Extend `smoke(..., expect_notice: true)` (run by
      `real_pty_accepts_chat_navigation_commands_and_restores_terminal`) to
      wait for the project's `*.staging-*` copy with `CREATING` on screen and
      to assert no later sentence replaced it.
- [x] 8.4 Raise the fixture-guard depth budget of spawned-binary fixtures
      whose own engine cache receives the store template
      (`test_support::TEMPLATE_DEPTH`; `cli.rs`, `terminal.rs`, `trust.rs`,
      `embedded_runtime.rs`).

## Evidence (2026-10-01, macOS arm64, shared loaded host, `MISE_LOCKED=1`)

- `//packages/kuru-memory:test`: exit 0; lib 515 passed, 0 failed, 4 ignored;
  the other targets 10, 5, 12 and 1 passed. The new
  `cancelled_open_during_template_build_finishes_and_leaves_a_ready_stage`
  was added after that run and passed in a `stage_worker::` run (3 passed).
- `//apps/kuru-tui:test`: exit 0 (every target; trust 22, terminal 44 with
  1 ignored, cli 43 with 1 ignored; passed counts 43 and 42). The first full run failed 3 trust tests
  on the fixture depth budget, fixed by 8.4 and rerun.
- `//packages/kuru-runtime:test`: exit 0, 220 passed.
- `lint` and `lint:windows` for kuru-memory and kuru-tui, `typecheck` for
  both, `format:check`, `docs:check`: exit 0.
- Mutation check for 8.2: reporting `WaitingForProjectOwnership` where the
  template path reports `OpeningDatabase` made the new PTY test fail with
  the waiting sentence named as having replaced the creating sentence;
  reverted.
- Not run locally: `//apps/kuru-tui:test:embedded-runtime` (packaged
  install and update acceptance; its depth-budget change is exercised only
  in CI), Linux and Windows native tests, coverage (CI gate).
