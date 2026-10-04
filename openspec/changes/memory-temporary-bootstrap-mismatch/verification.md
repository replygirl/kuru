# Verification

## 1. A foreign listener on the selected port never fails the bootstrap, and is never written to [critical]

- [~] 1.1 @regression (agent) `server_tests` foreign-listener test: a second real Dolt with the same store password and a different directory is started on the selected port from the port hook; the supervisor must reselect a port and report Ready for its own data directory, owned -> defer: to be observed at implementation; must FAIL on the current code with "Dolt bootstrap data directory mismatch" before the fix is applied and pass after
- [~] 1.2 @integration (agent) the same test asserts the foreign Dolt is still serving after the supervisor's shutdown and holds no bootstrap tables, user or grant written by the supervisor -> defer: to be observed at implementation
- [~] 1.3 @regression (agent) a foreign listener that persists across every attempt: the supervisor fails closed with the mismatch in its error context, publishes no Ready response and no endpoint -> defer: to be observed at implementation

## 2. Designed behaviour unchanged

- [~] 2.1 @integration (agent) `mise run //packages/kuru-memory:test` including `selected_port_takeover_retries_actual_dolt_without_touching_holder` and `persistent_selected_port_takeovers_exhaust_three_owned_attempts` -> defer: to be observed at implementation
- [~] 2.2 @unit (agent) the data-directory check still precedes every write and is not relaxed: reading the final diff shows `same_directory` and the `ensure!` unchanged -> defer: to be observed at review

## 3. Static checks

- [~] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run cospec -- validate --all --strict` -> defer: to be observed at implementation

## 4. Diagnosis record

- [x] 4.1 @manual (agent) job 111354197802 log read in full for the failing test -> observed: panic at `dolt_tests.rs:953:49`, causes "open active memory server", "memory supervisor exited unsuccessfully (exit status: 1)", "Dolt database bootstrap failed while checking the bootstrap data directory: Dolt bootstrap data directory mismatch"; `server.log` content is only "Kuru engine shutdown: Graceful"; 16 passed, 1 failed in 4.51 s. The compared identities are `files::directory(@@datadir).identity()` of the server that answered the authenticated probe and the identity of `<fixture directory>/data`. The cause (a foreign Dolt answering on the reserved port) is an inference from the code path, labelled as such in the proposal
