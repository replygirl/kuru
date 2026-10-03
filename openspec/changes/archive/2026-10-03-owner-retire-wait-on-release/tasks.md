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
