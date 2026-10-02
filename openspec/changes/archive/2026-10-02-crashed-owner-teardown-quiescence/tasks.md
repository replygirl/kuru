# Tasks

## 1. Diagnosis

- [x] 1.1 Record the job-log evidence, the test's teardown sequence, what PR #125 guarantees on each exit path and the exact leak point in `tmp/roadmap/store-creation-design/diag-crashed-owner-quiescence.md`, and verify by fetching job 110810405623 once and citing the server.rs and service.rs lines read
  - Fetched once with `gh api --allow-escape-sequences repos/replygirl/kuru/actions/jobs/110810405623/logs` (1588 lines). The lib binary started at 11:04:39.108 and the test FAILED at 11:05:00.735. Its `fixture_deadline(1, 1)` budget is over 96 s, so the body returned an early `Err`; the deadline did not elapse. The guard's panic (fixture_dir.rs:231) replaced that error. The unexplained store is the project directory and has "no quiescence record". The partition result was 158 passed and 1 failed.
  - Product reap verified by reading, not changed. server.rs `open_inner_with_probe_delay` spawns the supervisor with `process_group(0)`. `supervise_with_port_hook` takes the lifecycle lease at 2584-2620 and holds it to the end of the function. Lifetime EOF ends the run select (~2724), then `stop_child` runs (2739) and `observe_dolt` handles a failed stop (2756). A successor supervisor waits on the same lease.
  - Leak: on every early return, the test's root drops right after `KillServiceOnDrop` sends SIGKILL without waiting, while the orphaned supervisor is still stopping Dolt. The fixture is the defect, not the product. PR #125 (archived `2026-09-29-memory-lifecycle-ordering`) covered this fixture's success tail only. It named roots created inside timeout futures as an open follow-on.

## 2. Regression test (`packages/kuru-memory/src/service.rs`, tests module)

- [x] 2.1 Add an optional injected failure to the crash fixture and the test `crashed_owner_fixture_failure_is_reported_after_its_engine_quiesces` (inject after the sibling's write and liveness check, before the deliberate crash; assert the reported error is exactly the injected text and the root no longer exists), and verify that it fails on the old structure with the guard's panic, recording the exact command and output
  - **Measured on macOS arm64 (local), old structure.** The root and the single success-tail `await_managed_quiescence` were still inside the `timeout_at` body. Only the injection and the new test were added.
  - Command: `mise run //packages/kuru-memory:test -- service::tests::crashed_owner`, exit 101. Result: `crashed_owner_fixture_failure_is_reported_after_its_engine_quiesces ... FAILED` and `crashed_owner_retains_accepted_receipt_after_sibling_write ... ok`, which is "1 passed; 1 failed" in 3.40 s.
  - Panic, matching the CI signature: `panicked at packages/kuru-memory/src/test_support/fixture_dir.rs:231:13: fixture root …/kuru-fixture-OMeS2o/private (created by test service::tests::crashed_owner_fixture_failure_is_reported_after_its_engine_quiesces) was released without awaited memory quiescence, so it is kept in place. … store …/private/private/memory/1d6598b7…555c has no quiescence record`. The injected error never surfaced.

## 3. Fixture teardown on every exit path (`packages/kuru-memory/src/service.rs`, `packages/kuru-memory/src/test_support.rs`)

- [x] 3.1 Hoist the root and options out of the timed stage, run the stage through `FixtureDeadline::run`, then `settle` with `await_managed_quiescence` and `TempDir::release` on every path, removing the success-tail quiescence call, and re-export `settle`; verify by reading the diff (root not moved into the timed future, one quiescence call) and that no product file changed
  - `crashed_owner_receipt_fixture` creates the root, project and options before `deadline.run(async { … })`. The stage only borrows them. After the stage, `settle(stage.await, await_managed_quiescence(&options))` runs and then `root.release(outcome)`.
  - The success-tail `await_managed_quiescence` was removed, so it is called once per run. The elapsed-deadline error text is unchanged ("crashed-owner receipt fixture exceeded its … deadline").
  - `git diff --stat` touches only `packages/kuru-memory/src/service.rs` (`mod tests`) and one `cfg(test)` re-export line in `test_support.rs`. `server.rs`, `attach_or_spawn_elected`, `ORDINARY_POOL_WINDOW` and the readiness deadline are untouched.
- [x] 3.2 Confirm `retire_idle_service` and `await_store_quiescence` do not fail for an owner that was SIGKILLed without retiring its endpoint, and verify by reading `acquire_maintenance_permit_traced` (`NoEndpoint`/`PeerClosed` wait on the owner lock) and `Server::quiescence_at`, and by the regression test's exact-error assertion
  - With the owner dead, a stale endpoint answers `NoEndpoint` or `PeerClosed`. Neither is an error; both wait on the owner lock, which the kernel released at the owner's death. `quiescence_at` then waits up to `SUPERVISOR_REAP_ALLOWANCE` for the lease the orphaned supervisor holds until Dolt is reaped.
  - The regression test asserts `format!("{error:#}") == CRASH_FIXTURE_INJECTED`, so no teardown error or guard verdict was attached, and it passed (4.1).

## 4. Evidence

- [x] 4.1 Run the new test and `crashed_owner_retains_accepted_receipt_after_sibling_write` after the restructure, and verify that both pass, recording the command and durations
  - **Measured on macOS arm64 (local).** Command: `mise run //packages/kuru-memory:test -- service::tests::crashed_owner`, exit 0, 31 s wall including the build. Result: 2 passed, 0 failed, in 3.39 s. The fixture root is gone after the injected failure (`!root.exists()` asserted).
  - Repeated 5 more times: 2 passed each time, in 2.34 s, 1.87 s, 2.01 s, 2.08 s and 7.43 s.
  - The first post-fix attempt failed to link (`ld: write() failed, errno=28 (No space left on device)`), before any test ran. Running `mbx gc --max-size 30GiB` and then `--max-size 20GiB` freed space, and the run was repeated.
  - Source-scan and guard suites after the change: `mise run //packages/kuru-memory:test -- spawn_gate:: -- service::rpc::contract_tests test_support::served_owner test_support::fixture_dir`, exit 0, 46 passed.
- [x] 4.2 Run `format:check`, the memory package `lint` and `lint:windows`, and `typecheck`, and record each result, naming any check not run and why
  - `mise run format:check`: exit 0.
  - `mise run //packages/kuru-memory:lint`: exit 0 (24 s).
  - `mise run //packages/kuru-memory:typecheck`: exit 0 (22 s).
  - `mise run //packages/kuru-memory:lint:windows`: exit 0 (32 s).
  - Not run locally:
    - The full `//packages/kuru-memory:test` real-engine suite and `coverage`. The machine was shared with other builds and the disk was nearly full. CI runs both on this PR's head.
    - Native Windows and Linux behavior. These run in CI only.
