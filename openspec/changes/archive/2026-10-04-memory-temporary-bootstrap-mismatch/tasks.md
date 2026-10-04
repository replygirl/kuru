# Tasks

## 1. Regression test

- [x] 1.1 In `server_tests.rs`, add a test that starts a foreign authenticated Dolt on the supervisor's selected port from the port hook (same store password, different directory, bounded wait for it to accept connections, output drained), and verify it fails on the current code with "Dolt bootstrap data directory mismatch"
- [x] 1.2 Assert in the same test that the supervisor reselects a different port, reports Ready for its own data directory with an owned lifetime, publishes its identity, and that the foreign server is still serving and unwritten

## 2. Fix

- [x] 2.1 Make `initialize_database` return a typed data-directory mismatch error and have `start_database` close the pool, remember the mismatch and continue its loop so the owned Dolt's premature exit reaches the existing port-collision retry; verify the new test passes
- [x] 2.2 Verify a persistent foreign listener still fails closed: after the deadline or the finite attempt bound the supervisor errors with the mismatch in its context, publishes no Ready response, and wrote nothing to the foreign server
- [x] 2.3 Verify the existing selected-port collision tests (`selected_port_takeover_retries_actual_dolt_without_touching_holder`, `persistent_selected_port_takeovers_exhaust_three_owned_attempts`) still pass unchanged

## 3. Checks

- [x] 3.1 Run `mise run //packages/kuru-memory:test`, `mise run lint` and `mise run format:check`, and record the evidence in verification.md; name any unrun check and why
- [x] 3.2 Update the 2026-10-04 entry in the untracked flaky-test catalogue with the labelled cause
