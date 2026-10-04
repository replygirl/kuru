# Verification

## 1. The asking phase waits under the product's maintenance request budget [critical]

- [x] 1.1 @regression (agent) `the_asking_phase_outlives_the_startup_timeout_when_the_close_budget_is_larger`: `startup_timeout_secs = 1`, the owner lock held by an open still in progress that never answers, paused clock -> red with the asking bound reverted to `startup_timeout_secs`: expiry at `(1s; 1000ms since the first request)` against the expected `32s`; green with the fix: expiry at `maintenance_deadline` (the 32 s close budget) naming `asking phase`, the owner state `owner still opening; last stage = PreparingDatabase; no endpoint record present`, the trace and `active-client refusals=0`.
- [x] 1.2 @integration (agent) `an_elapsed_retirement_behind_an_unpublished_owner_names_its_open_stage`, a real in-process owner held at PreparingDatabase -> green with the fix; red under the reverted bound at `(30s; 30001ms ...)` against the expected 32 s.
- [x] 1.3 @integration (agent) `an_elapsed_retirement_behind_an_attached_client_names_its_request_budget`, a real served owner refusing an attached client over a socket -> green with the fix, naming `owner published` and nonzero busy replies; red under the reverted bound at `(1s; 1001ms ...)`. Twelve consecutive runs of this test and 1.1 on the built test binary passed.

## 2. The closing phase and the package are unchanged

- [x] 2.1 @unit (agent) `an_elapsed_retirement_bound_names_the_step_it_was_cancelled_in` and `an_elapsed_retirement_behind_a_held_close_names_a_closing_owner` -> still expire at the owner's close budget, unchanged and never reading as the asking phase.
- [x] 2.2 @integration (agent) `mise run //packages/kuru-memory:test`, the full package -> exit 0: 710 lib tests passed, 0 failed, 6 ignored, and every integration target passed.

## 3. Repository checks

- [x] 3.1 @integration (agent) `mise run format:check`, `lint`, `lint:windows`, `typecheck`, `docs:check` and `cospec -- validate --all --strict` -> exit 0 each (run with `env -u NODE_OPTIONS`; the shell's preload path is missing).
- [~] 3.2 @e2e (agent) native Linux and Windows jobs and the combined coverage gate -> defer: CI only, and no CI rerun is permitted for this change.
