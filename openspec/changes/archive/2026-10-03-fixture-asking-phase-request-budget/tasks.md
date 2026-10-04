# Tasks

## 1. Product budget as the asking bound

- [x] 1.1 Expose `service::maintenance_deadline` as `pub(crate)` and bound the fixture retirement's asking phase by it from the first request, leaving the closing-phase `close_budget` backstop unchanged. Verify no new constant exists and `grep startup_timeout_secs packages/kuru-memory/src/test_support.rs` no longer shows a fixture-chosen asking bound.
  - Evidence: `let asking = crate::service::maintenance_deadline(options);` in `retire_idle_service_traced`; the only change in `service.rs` is the visibility word. The remaining `startup_timeout_secs` hits in `test_support.rs` are the unrelated `default_startup` helper and doc text.
- [x] 1.2 Rename the expiry to name the phase and the budget, keeping the owner-state reading, the trace and the refusal count, and update the `retire_idle_service` doc comment and `docs/development.md`. Verify with grep that no `did not complete within memory.startup_timeout_secs` text remains.
  - Evidence: the expiry reads `managed owner retirement asking phase: no request found the owner closing within the maintenance request budget (<budget>; <N>ms since the first request); <owner state>; <trace>; active-client refusals=<R>`. A repository grep for `did not complete within memory.startup_timeout_secs` finds nothing outside archived change records.

## 2. Tests

- [x] 2.1 Replace `ensure_asking_deadline_expiry` with `ensure_asking_phase_expiry`, which derives the expected budget from `maintenance_deadline(options)` instead of a caller-supplied duration, and move the two activity tests that assert the asking expiry onto it (`an_elapsed_retirement_behind_an_attached_client_names_its_request_budget`, `an_elapsed_retirement_behind_an_unpublished_owner_names_its_open_stage`). Verify both pass.
  - Evidence: the attached-client test now runs on a paused clock after the client attaches (the refusals' socket replies complete before time advances), so it elapses the 32 s budget in about a second of wall time; it ran 12 consecutive times without a failure. `an_elapsed_retirement_bound_names_the_step_it_was_cancelled_in` asserts only the close-budget expiry and is unchanged.
- [x] 2.2 Add a regression test, `the_asking_phase_outlives_the_startup_timeout_when_the_close_budget_is_larger`, that fails before the fix and passes after: `startup_timeout_secs = 1`, an owner lock held by an open still in progress that never answers, paused clock, and an expiry asserted at `maintenance_deadline` (the close budget) that names the asking phase. No sleep in the assertion path.
  - Evidence, red: with the asking line temporarily reverted to `Duration::from_secs(startup_timeout_secs)` and the new text kept, the test failed with `...maintenance request budget (1s; 1000ms since the first request); owner still opening; last stage = PreparingDatabase...` against the expected `32s`. The unpublished-owner test failed at `(30s; 30001ms ...)` against the expected `32s`, and the attached-client test at `(1s; 1001ms ...)`. The edit was reverted; this is local evidence only and is not committed.
  - Evidence, green: with the fix, the same three tests and `a_retirement_that_began_while_the_owner_opened_retires_it_once_published` pass.

## 3. Checks

- [x] 3.1 Run the memory package tests and the repository checks and record observed results.
  - Evidence (local macOS arm64): `mise run //packages/kuru-memory:test` ran in full, exit 0: 710 lib tests passed, 0 failed, 6 ignored, and every integration target passed. `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check` and `mise run cospec -- validate --all --strict` exit 0. Mise tasks ran with `env -u NODE_OPTIONS` because the shell's NODE_OPTIONS preload points at a missing file and breaks the docs toolchain check.
  - Not run: coverage and native Linux and Windows jobs (CI only; no application code changed beyond a visibility keyword). No CI rerun is allowed for this change.
