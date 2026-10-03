# Tasks

## 1. Regression tests

- [x] 1.1 Add `an_elapsed_retirement_behind_an_unpublished_owner_names_its_open_stage` (owner held before publication at `PreparingDatabase`, paused time elapses the fixture bound) and verify it fails before the fix with the old text
- [x] 1.2 Add `an_elapsed_retirement_behind_a_held_close_names_a_closing_owner` (served owner held at `ClosePoint::AfterReap`; releasing the pause ends the close and a second retirement completes) and verify it fails before the fix

## 2. Fix

- [x] 2.1 Add the test-support `activity::inspect` and `describe_stage`, and `test_support::owner_state`, and append the owner's state to `retire_idle_service`'s expiry error; verify both regression tests and the existing elapsed-bound test (now also asserting the closing reading) pass three times
- [x] 2.2 Keep the flat 10 s and document on `retire_idle_service` why it cannot become an event wait yet (no outer backstop in `ServiceCleanup` or mise acceptance), and verify no caller's behaviour changes

## 3. Docs and checks

- [x] 3.1 Describe the expiry diagnostic and the missing backstop in `docs/development.md` and verify `mise run docs:check`
- [x] 3.2 Run the kuru-memory suite, the terminal test three times and the static checks, and record the results in verification.md

## 4. Review corrections

- [x] 4.1 Require the neutral prefix `managed owner retirement did not complete within 10 seconds`, no `idle managed owner`, and `no endpoint record present` in the three elapsed-bound tests, and verify they fail with the old text
- [x] 4.2 Change `retire_idle_service`'s expiry prefix and the opening reading's endpoint clause, refresh the `docs/development.md` sentence that quotes them, and verify the three tests pass three times
- [x] 4.3 Relabel the occurrence's "published" reading as inferred (Drop panic discards the test's outcome; option A not excluded) and record the undelivered close-phase stamps in the proposal

## 5. Decision (b): wait on the owner lock release under the close budget

- [x] 5.1 Add `a_retirement_behind_a_close_held_past_ten_seconds_completes_within_its_close_budget` (owner held at `ClosePoint::AfterReap` past the former 10 s, released within the close budget) and make the three elapsed-bound tests require the close budget's expiry (budget, time since the first closing reading, owner state); verify all four fail on 20e6189d's flat 10 s
- [x] 5.2 Stamp the first closing reply on `MaintenanceTrace` (test-support only) and replace the flat 10 s in `retire_idle_service`: ask with the active-client refusal loop (first under a retained 10 s; see 5.5), then wait on `await_owner_release` on its own thread and runtime under `close_budget()` from the first closing reading, then take the permit under the same backstop; verify the four tests pass three times and the full kuru-memory suite passes
- [x] 5.3 Confirm `ServiceCleanup::retire` (PTY and ConPTY) and mise acceptance `retire_blocking` are bounded by the retirement's own backstop, document it on both, update `docs/development.md`, and record option (c) as a separate PR
- [x] 5.4 Run the terminal test three times and the static checks, and record the results in verification.md
- [x] 5.5 Remove the retained 10 s asking bound: carry one `memory.startup_timeout_secs` deadline from the first request across the active-client refusal retries, report an attempt failing at it as the same expiry with owner state and trace, add `an_elapsed_retirement_behind_an_attached_client_names_its_startup_deadline` (red on the retained 10 s), and update `docs/development.md` and this record
- [x] 5.6 Make a no-endpoint reply a closing reading only when the owner's records show it is not opening (no endpoint record; activity record absent or marked failing), keep asking an opening owner under the asking deadline, add `a_retirement_that_began_while_the_owner_opened_retires_it_once_published` (red on 0a7188b4), move #197's held-open test to the asking deadline's expiry, and record the deviation and residual misreadings in the proposal, `docs/development.md` and this record
