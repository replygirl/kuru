# Tasks

Per site, record done (with its shape: derived from which product value, or inner bound deleted for the enclosing `FixtureDeadline`, or event observed) or not done (with the reason found in code). Line numbers are on origin/main 449dca9e; re-grep before editing, since PR #208 shifts `service.rs` below 1441 by 25 lines if it merges first. Every new bound names its product source in a comment or doc comment; no literal without a derivation, no retry, no change to `startup_timeout_secs`.

## 1. packages/kuru-memory/src/facade.rs

- [ ] 1.1 m2#4: replace the `Duration::from_secs(10)` `reap_within` of `served.retire(..)`, `served.restart(..)` and `served.reap(..)` at 19 sites (3308, 3775, 4018, 4214, 4651, 4855, 4962, 5102, 5470, 5649, 5699, 5855, 5937, 6101, 6232, 6350, 6428, 6527, 6612) with `crate::server::close_budget()` (32 s), passed by the caller and derivation stated once where the sites can see it, keeping the `ServedOwner` signatures and `test_support*` untouched; verify by reading the diff for no remaining 10 s reap bound in `facade.rs` and no edit under `src/test_support*`
  Evidence: pending.
- [ ] 1.2 m2#6: replace the `timeout(10 s, poll loop)` at 16 sites (3398, 3459, 3539, 3615, 3914, 4164, 4303, 4379, 4567, 4806, 4925, 5043, 5294, 5559, 5792, 6018) with `QUERY_TIMEOUT` (30 s, the write budget a sibling probe may wait for), or delete the inner timeout where an enclosing `FixtureDeadline::start(fixture_deadline(..))` bounds the same fixture and reports the hang as well, keeping 5559's fail-fast on the served owner exiting; verify by reading the diff for each site's shape and by the narrow test filters for the touched tests
  Evidence: pending.
- [ ] 1.3 m2#7: replace the `timeout(10 s, retry loop)` at 5586 (reconnect while the peer is closed), 5837 (`acquire_maintenance_permit` until active clients drain) and 6043 (`abandon_candidate_ref` until the Active refusal clears, retried with `yield_now`) with `maintenance_deadline(&options)` (32 s); verify by reading the diff and running the three tests
  Evidence: pending.
- [ ] 1.4 m2#15a: replace the 250 ms absence window at 6673 with the observable event of the owner's first `acquired: false` reply to the waiting second dream, using an existing event or a cfg(test) seam of a few lines in an owned file, then assert the waiter is still pending; if no seam of a few lines exists, record the site as deferred with the reason; verify by a test run, and by temporarily letting the second dream acquire (recorded, not committed) to see the test fail
  Evidence: pending.
- [ ] 1.5 m2#10: pass `Some(maintenance_deadline(&options))` (32 s) as `permit_within` at 6733 and 6949 instead of `Some(Duration::from_secs(20))`; verify by reading the diff and running the two tests
  Evidence: pending.
- [ ] 1.6 m2#13: replace the `timeout(5 s, first.revision())` at 6813 with `OPERATION_TIMEOUT` (35 s, the call's reply deadline, above `QUERY_TIMEOUT` 30 s) through `crate::service::OPERATION_TIMEOUT` once exception (b) lands, or delete it for the enclosing `FixtureDeadline`, keeping the failure message; verify by reading the diff and running the test
  Evidence: pending.

## 2. packages/kuru-memory/src/provision/tests.rs

- [ ] 2.1 m5#22: replace `cache_lock(.., Duration::from_secs(10))` after the release signal at 1039, 1127, 1184 and 1792 with `LOCK_TIMEOUT` (provision's own lock budget, 180 s, above `VERSION_TIMEOUT` 15 s that the detached cold-probe thread may hold the lease for), confirming from code that each wait is for that lease; verify by reading the diff and running the four tests
  Evidence: pending.
- [ ] 2.2 m5#24: derive the 5 s `started`-marker deadline at 1090 and 1093 from `VERSION_TIMEOUT` (15 s, the probe's own budget) for both callers (1105, 1156); verify by reading the diff and running the two tests
  Evidence: pending.

## 3. packages/kuru-memory/src/server_tests.rs

- [ ] 3.1 m4#56s: replace the 30 ms `quiescence(.., from_millis(30))` absence check at 241 with a causal one: the releaser task signals once it has returned from `observe_dolt`'s error path, then the test asserts the lease is still held; if that needs more than a cfg(test) seam of a few lines, record it as deferred; verify by a test run, and by temporarily dropping the lease early in the task (recorded, not committed) to see the test fail
  Evidence: pending.
- [ ] 3.2 m4#60: replace the `timeout(10 s, supervisor)` at 732 and 850 with `SUPERVISOR_REAP_ALLOWANCE` (13 s, above `CLOSE_GRACE` 8 s plus `KILL_GRACE` 3 s plus the 1 s output drain); verify by reading the diff and running the two tests
  Evidence: pending.

## 4. packages/kuru-memory/src/service.rs (test module and cfg(test) fixture code)

- [ ] 4.1 m1#5: derive `FixtureLoggedOwner::wait_for_exit`'s 10 s at 1532 and 1543 from `close_budget()` (32 s, the owner's own retire bound after the attachment closes), or `exited(fixture_deadline(..))` on unix; verify by reading the diff and, since its one caller is `apps/kuru-tui/tests/cli.rs:1266`, by a kuru-tui test compile
  Evidence: pending.
- [ ] 4.2 m1#36: replace the 5 s purge bound at 5482 and 5483 with `maintenance_deadline(&options)` (32 s) inside the outer `fixture_deadline(1, 0)`; verify by reading the diff and running the test
  Evidence: pending.
- [ ] 4.3 m1#37: replace the 20 s at 5509, 5510 (`timeout` of `acquire_maintenance_permit`) and 5544 (`permit_within` of `retire`) with `maintenance_deadline(&options)` (32 s); verify by reading the diff and running the test
  Evidence: pending.
- [ ] 4.4 m1#51: replace the `timeout(10 s, join)` at 6677, 7293, 7335, 7410, 7546 and 7571 with `QUERY_TIMEOUT` (30 s, via the write budget), keeping the result checks; verify by reading the diff and running the six tests
  Evidence: pending.
- [ ] 4.5 m1#53: replace the 10 s `owner.close()` bound at 6703 with `close_budget()` (32 s); verify by reading the diff and running the test
  Evidence: pending.
- [ ] 4.6 m1#54: replace `Served::finish`'s 20 s at 6736 with `QUERY_TIMEOUT` (30 s, one request round trip), one edit for its 16 call sites (6818, 6940, 6971, 6981, 7024, 7033, 7089, 7109, 7119, 7190, 7381, 7386, 7448, 7490, 7496, 7582); verify by reading the diff and running the settlement tests
  Evidence: pending.
- [ ] 4.7 m1#56: replace `SettlementFixture::close`'s 20 s at 6855 with `close_budget()` (32 s), one edit for the 10 tests that use `settled_fixture`; verify by reading the diff and running them
  Evidence: pending.
- [ ] 4.8 m1#57: replace `next_wait_event`'s 10 s at 6890 with `QUERY_TIMEOUT` (30 s, the settlement wait budget of rpc.rs 897-908), one edit for its 17 call sites; verify by reading the diff and running the settlement tests
  Evidence: pending.
- [ ] 4.9 m1#59: replace the `timeout(10 s, <pause>.entered.notified())` at 6925, 6964, 7005, 7078, 7097, 7168, 7359, 7406, 7435, 7468 and 7526 with `QUERY_TIMEOUT` (30 s, the owner's write precedes the pause), or `fixture_deadline(0, 0)`; leave the 5 s at 6663 (a remainder row) unless it sits on an edited line; verify by reading the diff and running the 11 tests
  Evidence: pending.
- [ ] 4.10 m1#64: replace the `timeout(10 s, poll loop)` at 7792 with `QUERY_TIMEOUT` (30 s) and at 8261 and 8323 with `HANDLER_BUDGET` (30 s, widened to `pub(crate)` under exception (a), no value change), polling the durable outcome as before; verify by reading the diff and running the three tests
  Evidence: pending.

## 5. packages/kuru-memory/src/service/rpc.rs (cfg(test) code and one visibility edit)

- [ ] 5.1 m2#29: replace the `timeout(10 s, pause.release.notified())` in `pause_after_registration` (1006), and with it m2#30 (`pause_before_settlement`, 1023) and m2#31 (`pause_before_dispatch`, 1179), with `OPERATION_TIMEOUT` (35 s, same file) unless the code shows a tighter enclosing handler deadline, which is then recorded and used, and make `OPERATION_TIMEOUT` `pub` with `#[cfg(any(test, feature = "test-support"))] pub use rpc::OPERATION_TIMEOUT;` next to the existing `pub use rpc::{..}` at `service.rs:28` (exception (b), no value change, the next PR depends on it); verify by reading the diff for no value change, by the 8 tests that use `progress.pause_next(..)` (service.rs 6621, 6959, 6996, 7094, 7401, 7463, 7517, 9198) and by a kuru-tui compile
  Evidence: pending.

## 6. packages/kuru-memory/src/spawn_gate.rs (tests)

- [ ] 6.1 m4#79: replace the 200 ms `timeout(.., &mut acquired_seen)` absence check at 490 with waiting until the taker is queued (`GATE.lock.try_read()` fails, as the sibling tests' `queued_seen` do) and then asserting it has not acquired; verify by a test run, and by temporarily letting the taker acquire during creation (recorded, not committed) to see the test fail
  Evidence: pending.

## 7. packages/kuru-memory/src/store.rs (test module)

- [ ] 7.1 m3#55: replace the `timeout(10 s, pause.*.reached.notified())` at 12799, 12800, 12813 and 12814 with `QUERY_TIMEOUT` (30 s, the one write budget the receipt insert and `DOLT_COMMIT` spend); verify by reading the diff and running the two tests
  Evidence: pending.
- [ ] 7.2 m3#56: replace the `timeout(10 s, store.probe_logical_receipt(..))` at 12805, 12806, 12819 and 12820 with `QUERY_TIMEOUT` (30 s, `operation_receipt_matches` runs under it); verify by reading the diff and running the two tests
  Evidence: pending.

## 8. packages/kuru-memory/src/store/migration_lifecycle_tests.rs

- [ ] 8.1 m3#80: replace the `timeout(6 s, spawn_gated_open(options))` at 256 and 257 with a bound written out as the configured 1 s startup plus `SUPERVISOR_TRANSPORT_ALLOWANCE` (2 s) plus the `finish_owner` deadline `SUPERVISOR_REAP_ALLOWANCE` (13 s), derived from the options' own `startup_timeout_secs`; verify by reading the diff for the stated derivation and running the test
  Evidence: pending.
- [ ] 8.2 m3#75: delete the local `DEADLINE` (48, 10 s) and derive each use from `close_budget()` (the orphaned supervisor's stop of 11 s and reap of 13 s), with m3#77 (111), m3#78 (131, 177) and m3#79 (213, `Server::quiescence_at(.., DEADLINE)`) fixed by the same edit; verify by `rg DEADLINE` finding no constant left in the file and by running the lifecycle tests
  Evidence: pending.

## 9. packages/kuru-memory/src/store/migrations.rs (tests)

- [ ] 9.1 m5#57: replace the `timeout(10 s, poll public_transcript_page)` at 4994 with `QUERY_TIMEOUT` (30 s, the statements run under `pool::within(QUERY_TIMEOUT)`); verify by reading the diff and running the test
  Evidence: pending.

## 10. packages/kuru-memory/src/store/operational_gc_tests.rs

- [ ] 10.1 m3#32: delete `TEST_DEADLINE` (line 4, 10 s) and derive each of its 25 uses from the product value it waits on, `QUERY_TIMEOUT` for candidate operations, statements and joins and the startup budget for opens, with m3#36 to m3#40, m3#42 and m3#43 fixed by the same edit (every `rg -n TEST_DEADLINE` hit; the inventory's statement lines are 122, 213, 380, 383, 420, 422, 465, 519, 527, 531, 532, 534, 844, 854, 855, 910, 911, 974, 975, 1031, 1091, 1146, 1192, 1193, 1235, 1238, 1379); verify by `rg TEST_DEADLINE` finding nothing in the file, by reading the diff for each use's derivation and by running the file's tests
  Evidence: pending.
- [ ] 10.2 m3#33: replace the 80 ms absence windows at 99 (put), 184 (abandon) and 521 (new source pool) with an observable event inside the wait (reconcile parked on the live session, pool call parked on the fence) before asserting not finished, noting that at 521 `attempt_started` fires before `server.pool()` is awaited; a cfg(test) seam of a few lines is allowed, more is recorded as deferred; verify by a test run, and by temporarily releasing the fence early (recorded, not committed) to see the test fail
  Evidence: pending.

## 11. packages/kuru-memory/src/store/recovery_tests.rs

- [ ] 11.1 m3#16: replace the 80 ms `timeout(.., spawn_gated_open(options))` absence check at 1181 and 1182 with waiting for the opener's `MemoryOpenStage::WaitingForProjectOwnership` progress report, then asserting it is still pending; verify by a test run, and by temporarily letting the opener acquire (recorded, not committed) to see the test fail
  Evidence: pending.
- [ ] 11.2 m3#17: replace the 80 ms `timeout(.., store.reconcile())` absence check at 1255, and with it m3#18 (the 50 ms checks at 1299 and 1577), with an observable event that reconcile entered the session wait, then assert not finished; a cfg(test) seam of a few lines is allowed, more is recorded as deferred; verify by a test run
  Evidence: pending.
- [ ] 11.3 m3#1: delete `TEST_DEADLINE` (line 25, 10 s) and derive each of its 40 uses from the product value it waits on (`QUERY_TIMEOUT`, `close_budget()` or `migration_observation_deadline`), with m3#6 (317), m3#7 (367, 369, 378, 380, including the Windows `child.wait(..)`), m3#8 (390, 398), m3#11 (551), m3#20, m3#21, m3#23 (the `await_flag` callers), m3#25 (1329) and m3#26 (3557) fixed by the same edit; verify by `rg TEST_DEADLINE` finding nothing in the file, by reading the diff for each use's derivation, by running the file's tests and by the Windows-target lint for the `cfg(windows)` uses
  Evidence: pending.
- [ ] 11.4 m3#22: replace `TEST_DEADLINE` at 2508, 2610, 2745, 2759, 2765, 2904, 3016 and 3020 with `migration_observation_deadline(&options)` (startup plus `QUERY_TIMEOUT`, already used by the neighbouring waits); verify by reading the diff and running the migration tests
  Evidence: pending.

## 12. packages/kuru-memory/tests/fixtures/parent/windows.rs

- [ ] 12.1 m4#114: replace the `timeout(5 s, ..)` at 119 with `QUERY_TIMEOUT` (30 s, the pool acquire ceiling at server.rs:1119), subject to the visibility ruling in the proposal: the fixture is a separate crate and `QUERY_TIMEOUT` is `pub(crate)`, so either a cfg-gated `pub` re-export (needs the lead's ruling, edits an unowned file) or record the site as deferred with that reason; cannot run here, so verify by `mise run //packages/kuru-memory:lint:windows`
  Evidence: pending.

## 13. packages/kuru-memory/tests/server_lifecycle.rs

- [ ] 13.1 m4#88: replace the 150 ms `timeout(.., pending.as_mut())` absence check at 259 with observing the writable open at its lease wait (a new event, or a single poll once the cold reader holds the lease), then asserting it is still pending; a seam of a few lines in an owned file is allowed, else record as deferred; verify by a test run
  Evidence: pending.
- [ ] 13.2 m4#84: derive `options()`'s 20 s `timeout` at 40 from the default startup (30 s) as `OpenOptions::new(..).config.startup_timeout_secs`, which is public and exposes nothing new, for its 11 call sites (79, 114, 238, 250, 305, 336, 371, 488, 514, 539, 595); verify by reading the diff and running `server_lifecycle`
  Evidence: pending.
- [ ] 13.3 m4#90: replace the finite `sleep(60 s)` keep-alive at 542 with a peer that blocks until released (a read on a never-written stdin), so the helper stays alive until the test kills it; verify by running the test and reading that no sleep literal remains
  Evidence: pending.
- [ ] 13.4 m4#93: replace the 5 s deadline at 652 and 657 for the supervisor to exit after SIGTERM with `SUPERVISOR_REAP_ALLOWANCE` (13 s), subject to the visibility ruling in the proposal; either a cfg-gated `pub` re-export (needs the lead's ruling) or record the site as deferred with that reason; verify by reading the diff and running the test
  Evidence: pending.
- [ ] 13.5 m4#87: replace the `timeout(10 s, SELECT SLEEP(6))` at 92 and 93 with `QUERY_TIMEOUT` (30 s, the pool's `acquire_timeout`), or with `options.timeout` once m4#84 sets it to the 30 s default startup, subject to the visibility ruling in the proposal for `QUERY_TIMEOUT`; keep the 6 s SQL `SLEEP` stimulus (it must exceed Dolt's 5 s timer); verify by reading the diff and running the test
  Evidence: pending.

## 14. packages/kuru-memory/tests/windows_lifecycle.rs

- [ ] 14.1 m4#103: replace `child.wait(Duration::from_secs(15))` at 396 and 785 with `close_budget()` (32 s, the creator's normal real-Dolt close), subject to the visibility ruling in the proposal; read 556 and 626 from the code (the inventory records frame-after-cleanup and `TerminateProcess` exits at once, so no close is enclosed) and leave them unchanged with that reason if confirmed; cannot run here, so verify by `mise run //packages/kuru-memory:lint:windows`
  Evidence: pending.

## 15. Checks

- [ ] 15.1 Run `mise run //packages/kuru-memory:test` and the narrower filters used while iterating, and record pass counts and any failure with its cause; state that local runs do not reproduce a loaded CI runner
  Evidence: pending.
- [ ] 15.2 Run `mise run //packages/kuru-memory:lint`, `mise run //packages/kuru-memory:lint:windows`, `mise run format:check` and `mise run typecheck` and record each exit code
  Evidence: pending.
- [ ] 15.3 Verify the diff touches only the owned files and this change's openspec directory (and not `mise.lock`, nor `src/test_support*`), and that no new `from_secs(` or `from_millis(` literal lacks a derivation, by reading `git diff --stat origin/main` and searching the diff for added literals
  Evidence: pending.
- [ ] 15.4 Validate with `mise run cospec -- validate memory-derived-waits --strict`, confirm the apply gate is clear, and archive before the final commit, confirming the archive exists
  Evidence: pending.
- [ ] 15.5 CI evidence: the native test jobs on Linux, macOS and Windows run the changed memory tests to green, including the Windows-only sites; awaits CI
  Evidence: awaits CI.
- [ ] 15.6 CI evidence: the combined coverage run holds the 90% workspace line gate with the changed tests; awaits CI
  Evidence: awaits CI.
