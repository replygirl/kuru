# Tasks

## 1. Derivation table and shared launch budget

- [ ] 1.1 Write the derivation table, one row per fix point (22 rows, ids d1#63, d1#77, d1#87, d1#95, d1#99, d1#101, d1#108, d1#109, d1#110, d2#1, d2#2, d2#5, d2#8, d2#11, d2#19, d2#24, d2#26, d2#41, d2#47, d2#58, d2#64, d2#74), naming the governing product budget or event and the arithmetic, and verify every row is either done or deferred with a reason
- [ ] 1.2 Extend `tests/support/launch_budget.rs` with the budgets the remaining copies need, each with its derivation at the constant, and verify a pin test states the arithmetic

## 2. Launch-budget families (tests/)

- [ ] 2.1 Replace `TIMEOUT` in `bootstrap_install.rs` and `bootstrap_windows.rs`, `TEST_TIMEOUT` in `release_notes.rs`, and `TIMEOUT` in `windows_update.rs` (d2#24, d2#41, d2#47, d2#58) with the shared budget and verify the files compile and their tests pass
- [ ] 2.2 Replace the literals in `advisory.rs`, `bundle_prepare.rs`, `support/repository_environment.rs` and `support/fixture_git.rs` `BOUND` (d2#1, d2#2, d2#19, d2#74, d2#8) and verify the same
- [ ] 2.3 Start the `advisory.rs` 250 ms clocks after the observed ready marker (d2#5) and verify the timeout path still fires and is asserted

## 3. Ceiling undercuts in the bundle tests (src test modules)

- [ ] 3.1 Derive the `recovery_tests.rs`, `build_tests.rs` and `bundle.rs` test-module bounds from DOWNLOAD_TIMEOUT, RETRY_DELAYS[0] and the ICU download budget, or await the event (d1#63, d1#95, d1#99, d1#101, d1#108, d1#110) and verify no literal shorter than the product ceiling remains
- [ ] 3.2 Derive the 600 ms absence window from DEADLINE_RETRY_DELAYS[0] with a written multiple (d1#109) and verify the arithmetic is pinned

## 4. paced_http idle races and keep-alives

- [ ] 4.1 Replace the 200 ms idle races in `archive/tests.rs` and the `published.rs` test module with header-delivery signalling or a derived multiple (d1#87, d1#77) and verify the tests assert the same outcomes
- [ ] 4.2 Replace the finite `sleep 60` keep-alives in `bootstrap_install.rs` and `fixtures/delivery.rs` with peers that block until released (d2#26, d2#11) and verify the group cleanup tests still pass and nothing outlives them

## 5. windows_update late-header clock

- [ ] 5.1 Make the late-header test independent of a 2 s sleep against a 3 s budget through an injectable clock (d2#64), or record it as the one deferred item with its reason, and verify which

## 6. Verification

- [ ] 6.1 Run the kuru-delivery test, lint (host and Windows target), format and typecheck tasks and the cospec checks, and record observed results and unrun checks with reasons
- [ ] 6.2 Run `mise run cospec -- validate delivery-test-waits --strict`, open the PR, and record CI evidence from job logs for the final head
