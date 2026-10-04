# Tasks

Per site, record done (with its shape: derived from which product value, or inner bound deleted for the enclosing `FixtureDeadline`, or event observed) or not done (with the reason found in code). Line numbers are on origin/main 449dca9e; re-grep before editing, since PR #208 shifts `service.rs` below 1441 by 25 lines if it merges first. Every new bound names its product source in a comment or doc comment; no literal without a derivation, no retry, no change to `startup_timeout_secs`.

Exception (b) landed first and alone, as 3fa5318e `test(memory): expose product budgets to test-support consumers`: the new `src/test_budgets.rs` (forwarding `OPERATION_TIMEOUT`, `QUERY_TIMEOUT`, `close_budget()` and `SUPERVISOR_REAP_ALLOWANCE`, each with a doc comment naming its source), its `lib.rs` declaration, and the one widening it needs, `service::rpc::OPERATION_TIMEOUT` from private to `pub(crate)`. `QUERY_TIMEOUT`, `close_budget()` and `SUPERVISOR_REAP_ALLOWANCE` were already `pub(crate)` and `rpc` already `pub(crate) mod`, so nothing else was widened. No value changed. `HANDLER_BUDGET` was not widened (see 4.10).

Mutation checks: the tasks below that ask for a temporary product mutation to watch a causal check fail (1.4, 3.1, 6.1, 11.1) were not run. The temporary edits (the dream-lease reply, `observe_dolt`'s return, the gate's drain and the startup-lock wait) were refused by the local permission system, and no workaround was attempted. Each of those rows is verified by its passing run only.

## 1. packages/kuru-memory/src/facade.rs

- [x] 1.1 m2#4: replace the `Duration::from_secs(10)` `reap_within` of `served.retire(..)`, `served.restart(..)` and `served.reap(..)` at 19 sites (3308, 3775, 4018, 4214, 4651, 4855, 4962, 5102, 5470, 5649, 5699, 5855, 5937, 6101, 6232, 6350, 6428, 6527, 6612) with `crate::server::close_budget()` (32 s), passed by the caller and derivation stated once where the sites can see it, keeping the `ServedOwner` signatures and `test_support*` untouched; verify by reading the diff for no remaining 10 s reap bound in `facade.rs` and no edit under `src/test_support*`
  Evidence: done. All 19 sites pass `owner_reap_within()`, a test-module fn returning `crate::server::close_budget()`, whose doc comment states the derivation (the owner task returns when its store close does). `ServedOwner` and `src/test_support*` are untouched. Remainder reap literals on other lines stay as they were (the 10 s reap at 5563 inside m2#6's 5559 block and the 5 s `reap_within` beside m2#10).
- [x] 1.2 m2#6: replace the `timeout(10 s, poll loop)` at 16 sites (3398, 3459, 3539, 3615, 3914, 4164, 4303, 4379, 4567, 4806, 4925, 5043, 5294, 5559, 5792, 6018) with `QUERY_TIMEOUT` (30 s, the write budget a sibling probe may wait for), or delete the inner timeout where an enclosing `FixtureDeadline::start(fixture_deadline(..))` bounds the same fixture and reports the hang as well, keeping 5559's fail-fast on the served owner exiting; verify by reading the diff for each site's shape and by the narrow test filters for the touched tests
  Evidence: done. 15 sites use `OWNER_COMMIT_WITHIN` = `store::QUERY_TIMEOUT`; its doc comment states that the owner commits the accepted write within its write budget (`write_deadline`). 5559 takes the other shape: its inner bound is deleted for the enclosing `FixtureDeadline` (facade.rs:5485 on the base). It polls for the selected abandonment's ref deletion, which the owner reaches through several statements of a multi-step candidate operation (`store.abandon_candidate_ref`, store.rs:4228) with no single product budget. Its fail-fast on the served owner exiting and its `last_status` context are kept.
- [x] 1.3 m2#7: replace the `timeout(10 s, retry loop)` at 5586 (reconnect while the peer is closed), 5837 (`acquire_maintenance_permit` until active clients drain) and 6043 (`abandon_candidate_ref` until the Active refusal clears, retried with `yield_now`) with `maintenance_deadline(&options)` (32 s); verify by reading the diff and running the three tests
  Evidence: done, with a code-backed difference at two sites. 5837 waits on `acquire_maintenance_permit` and uses `service::maintenance_deadline(&options)`. 5586 and 6043 wait on no maintenance step, so their inner bounds are deleted for the enclosing `FixtureDeadline` (facade.rs:5485 and 5958 on the base). At 5586 the owner drops its candidate-resolution reservation when the `AbandonCandidateRef` handler returns (rpc.rs:2364-2367), after a multi-step candidate operation. At 6043 each refused attempt is a full round trip under the client's reply deadline, and the accepted attempt is that same multi-step operation. Comments at both sites say so.
- [x] 1.4 m2#15a: replace the 250 ms absence window at 6673 with the observable event of the owner's first `acquired: false` reply to the waiting second dream, using an existing event or a cfg(test) seam of a few lines in an owned file, then assert the waiter is still pending; if no seam of a few lines exists, record the site as deferred with the reason; verify by a test run, and by temporarily letting the second dream acquire (recorded, not committed) to see the test fail
  Evidence: done with a cfg(test) seam of 7 lines in facade.rs: a `dream_lease_refused: Notify` on `RemoteSession`, its initializer, and `notify_one()` in the `acquired: false` arm of `MemoryStore::acquire_dream_lease`. Forks share the session, so the waiter's refusal is observed on `second`'s session. The test selects that refusal against the waiter finishing (finishing fails the test) and then asserts the waiter is unfinished. No window remains. The 250 ms window at 6700 is a remainder row on another line and is unchanged. The temporary mutation was not run (see the note above).
- [x] 1.5 m2#10: pass `Some(maintenance_deadline(&options))` (32 s) as `permit_within` at 6733 and 6949 instead of `Some(Duration::from_secs(20))`; verify by reading the diff and running the two tests
  Evidence: done, `Some(service::maintenance_deadline(&options))` at both.
- [x] 1.6 m2#13: replace the `timeout(5 s, first.revision())` at 6813 with `OPERATION_TIMEOUT` (35 s, the call's reply deadline, above `QUERY_TIMEOUT` 30 s) through `crate::service::rpc::OPERATION_TIMEOUT`, widened to `pub(crate)` by exception (b), or delete it for the enclosing `FixtureDeadline`, keeping the failure message; verify by reading the diff and running the test
  Evidence: done, `service::rpc::OPERATION_TIMEOUT`, with a comment that an independent read answers within its own reply deadline and a serialized one never would. The failure message is unchanged.
  Run: `mise run //packages/kuru-memory:test -- facade::tests`: 27 passed, 0 failed, 60.7 s.

## 2. packages/kuru-memory/src/provision/tests.rs

- [x] 2.1 m5#22: replace `cache_lock(.., Duration::from_secs(10))` after the release signal at 1039, 1127, 1184 and 1792 with the budget of the lease holder each waits on, confirming it from code; verify by reading the diff and running the four tests
  Evidence: done, by holder, per the lead's ruling to use `LOCK_TIMEOUT` only where the waiter can legitimately wait that long. At 1127 and 1184 the lease is held by the detached cold-probe thread until `verify_version_recorded(.., VERSION_TIMEOUT)` returns (`CheckedColdProbe::probe`, provision.rs:735-755), so they use `VERSION_TIMEOUT` (15 s); `LOCK_TIMEOUT` would overstate that holder. At 1039 and 1792 the holder is the released extraction worker, which has no time bound of its own, and the product's own waiter for this lock is `cache_lock(&cache, LOCK_TIMEOUT)` (provision.rs:140), so they use `LOCK_TIMEOUT` (180 s). A comment at each site states its holder.
- [x] 2.2 m5#24: derive the 5 s `started`-marker deadline at 1090 and 1093 from `VERSION_TIMEOUT` (15 s, the probe's own budget) for both callers (1105, 1156); verify by reading the diff and running the two tests
  Evidence: done, `VERSION_TIMEOUT`, with the derivation at the site.
  Run: `mise run //packages/kuru-memory:test -- -- provision::tests spawn_gate`: 42 passed, 0 failed, 8.8 s.

## 3. packages/kuru-memory/src/server_tests.rs

- [x] 3.1 m4#56s: replace the 30 ms `quiescence(.., from_millis(30))` absence check at 241 with a causal one: the releaser task signals once it has returned from `observe_dolt`'s error path, then the test asserts the lease is still held; if that needs more than a cfg(test) seam of a few lines, record it as deferred; verify by a test run, and by temporarily dropping the lease early in the task (recorded, not committed) to see the test fail
  Evidence: done, in the test alone. The status closure sends a second oneshot on its next call after the injected failure, which shows that `observe_dolt` went on observing the child. The test awaits it with no timer: had the observer returned on the failure, the task would end without sending, and the receive would fail. The 30 ms `quiescence` call stays after the barrier as one bounded try confirming the actual lease; it no longer decides the race. The temporary mutation was not run (see the note above).
- [x] 3.2 m4#60: replace the `timeout(10 s, supervisor)` at 732 and 850 with `SUPERVISOR_REAP_ALLOWANCE` (13 s, above `CLOSE_GRACE` 8 s plus `KILL_GRACE` 3 s plus the 1 s output drain); verify by reading the diff and running the two tests
  Evidence: done, with the derivation at both sites.
  Run: `mise run //packages/kuru-memory:test -- -- service:: server::tests`: 162 passed, 0 failed, 212 s, including `cleanup_observation_error_keeps_actual_lifecycle_lease_until_child_exit` and `persistent_selected_port_takeovers_exhaust_three_owned_attempts`.

## 4. packages/kuru-memory/src/service.rs (test module and cfg(test) fixture code)

- [x] 4.1 m1#5: derive `FixtureLoggedOwner::wait_for_exit`'s 10 s at 1532 and 1543 from `close_budget()` (32 s, the owner's own retire bound after the attachment closes), or `exited(fixture_deadline(..))` on unix; verify by reading the diff and, since its one caller is `apps/kuru-tui/tests/cli.rs:1266`, by a kuru-tui test compile
  Evidence: done, `crate::server::close_budget()`, with the derivation at the site. The signature is unchanged, so its kuru-tui caller is unaffected; `mise run typecheck` (all workspace targets) covers that compile.
- [x] 4.2 m1#36: replace the 5 s purge bound at 5482 and 5483 with `maintenance_deadline(&options)` (32 s) inside the outer `fixture_deadline(1, 0)`; verify by reading the diff and running the test
  Evidence: done; the failure message now names the maintenance deadline instead of five seconds.
- [x] 4.3 m1#37: replace the 20 s at 5509, 5510 (`timeout` of `acquire_maintenance_permit`) and 5544 (`permit_within` of `retire`) with `maintenance_deadline(&options)` (32 s); verify by reading the diff and running the test
  Evidence: done at all three; the adjacent 5 s `reap_within` lines are remainder rows and are unchanged.
- [x] 4.4 m1#51: replace the `timeout(10 s, join)` at 6677, 7293, 7335, 7410, 7546 and 7571 with `QUERY_TIMEOUT` (30 s, via the write budget), keeping the result checks; verify by reading the diff and running the six tests
  Evidence: done; all six use `OWNER_STEP_WITHIN` = `crate::store::QUERY_TIMEOUT`, whose doc comment gives the derivation (the request's write spends at most its write budget, and its serving task ends after it, or at once on an abort or a departed client). At 6677 the remainder 40 s outer bound at 6662 still encloses the new inner bound.
- [x] 4.5 m1#53: replace the 10 s `owner.close()` bound at 6703 with `close_budget()` (32 s); verify by reading the diff and running the test
  Evidence: done.
- [x] 4.6 m1#54: replace `Served::finish`'s 20 s at 6736 for its 16 call sites (6818, 6940, 6971, 6981, 7024, 7033, 7089, 7109, 7119, 7190, 7381, 7386, 7448, 7490, 7496, 7582); verify by reading the diff and running the settlement tests
  Evidence: done with `rpc::OPERATION_TIMEOUT` (35 s) rather than `QUERY_TIMEOUT`, a code-backed difference. `Served::finish` joins both halves of one round trip, for writes and outcome queries alike. The client half fails at its reply deadline, `OPERATION_TIMEOUT`. A write's own work (`QUERY_TIMEOUT`) and an outcome handler's `HANDLER_BUDGET` both fit inside it, leaving `REPLY_MARGIN` for the reply (rpc.rs:705-718). `QUERY_TIMEOUT` alone would leave no room for the reply. The doc comment on `finish` states this.
- [x] 4.7 m1#56: replace `SettlementFixture::close`'s 20 s at 6855 with `close_budget()` (32 s), one edit for the 10 tests that use `settled_fixture`; verify by reading the diff and running them
  Evidence: done.
- [x] 4.8 m1#57: replace `next_wait_event`'s 10 s at 6890, one edit for its 17 call sites; verify by reading the diff and running the settlement tests
  Evidence: done with `rpc::OPERATION_TIMEOUT`, a code-backed difference from the planned `QUERY_TIMEOUT`. Wait events come from an outcome handler, which answers within `HANDLER_BUDGET` of its entry (`gate_outcome`, rpc.rs:1830). Its settlement wait ends by `min(now + QUERY_TIMEOUT, deadline - REPLY_MARGIN - PROBE_BUDGET)`, so every event precedes the client's reply deadline from the query. The doc comment states this, and the failure message now names the reply deadline.
- [x] 4.9 m1#59: replace the `timeout(10 s, <pause>.entered.notified())` at 6925, 6964, 7005, 7078, 7097, 7168, 7359, 7406, 7435, 7468 and 7526 with `QUERY_TIMEOUT` (30 s, the owner's write precedes the pause), or `fixture_deadline(0, 0)`; leave the 5 s at 6663 (a remainder row) unless it sits on an edited line; verify by reading the diff and running the 11 tests
  Evidence: done; all 11 use `OWNER_STEP_WITHIN` (`QUERY_TIMEOUT`). The settlement pauses (6925, 7078, 7168, 7359, 7435) follow the write; the registered pauses (6964, 7005, 7097, 7406, 7468, 7526) precede it. The 5 s at 6663 is unchanged.
- [x] 4.10 m1#64: replace the `timeout(10 s, poll loop)` at 7792, 8261 and 8323, polling the durable outcome as before; verify by reading the diff and running the three tests
  Evidence: done. 7792 uses `crate::store::QUERY_TIMEOUT`: the owner commits the accepted write within its write budget. At 8261 and 8323 the inner bound is deleted for the enclosing `FixtureDeadline` (service.rs:8206), not replaced by `HANDLER_BUDGET`. Each outcome query answers within `HANDLER_BUDGET`, but the accepted promotion or abandonment is a multi-step candidate operation that may answer `InFlight` more than once, so `HANDLER_BUDGET` would undercut a legitimate second query. `HANDLER_BUDGET` was therefore not widened, and no product item changed visibility for this row. The context strings now read "accepted promotion proof" and "accepted abandonment proof".
  Run: see 3.2 (162 passed, including `lost_candidate_transition_replies_survive_sibling_write_and_owner_restart` and `outcome_wait_leaves_other_attachments_free`).

## 5. packages/kuru-memory/src/service/rpc.rs (cfg(test) code and one visibility edit)

- [x] 5.1 m2#29: replace the `timeout(10 s, pause.release.notified())` in `pause_after_registration` (1006), and with it m2#30 (`pause_before_settlement`, 1023) and m2#31 (`pause_before_dispatch`, 1179), with `OPERATION_TIMEOUT` (35 s, same file) unless the code shows a tighter enclosing handler deadline, which is then recorded and used; verify by reading the diff for no value change and by the 8 tests that use `progress.pause_next(..)` (service.rs 6621, 6959, 6996, 7094, 7401, 7463, 7517, 9198)
  Evidence: done; all three use a cfg(test) `TEST_PAUSE_RELEASE_WITHIN` = `OPERATION_TIMEOUT`. No enclosing handler deadline wraps them: the pauses sit in `serve_attached` (rpc.rs:1349), `respond` (1438) and `process` (1535), none under a timeout. The doc comment gives the derivation: the paused request's client gives up on its reply at its reply deadline, so a test that held the pause longer could not observe that reply anyway. The failure messages now name the reply deadline; no test matched the old text. The visibility edit is exception (b) in 3fa5318e (see the note at the top), with no value change.

## 6. packages/kuru-memory/src/spawn_gate.rs (tests)

- [x] 6.1 m4#79: replace the 200 ms `timeout(.., &mut acquired_seen)` absence check at 490 with waiting until the taker is queued (`GATE.lock.try_read()` fails, as the sibling tests' `queued_seen` do) and then asserting it has not acquired; verify by a test run, and by temporarily letting the taker acquire during creation (recorded, not committed) to see the test fail
  Evidence: done, in the inventory's shape. A `yield_now` loop without a timer waits until `GATE.lock.try_read()` fails, which means the taker holds the write lock and is draining. The write is uncontended, so the loop ends unless the taker thread has ended, which it asserts. `acquired_seen.try_recv()` must then be empty. Residual, not a window: between the taker's write-lock acquisition and its drain there is no further observable without a seam, so a drain that returned at once could still send after the check. The temporary mutation was not run (see the note above). Run: see 2.2.

## 7. packages/kuru-memory/src/store.rs (test module)

- [x] 7.1 m3#55: replace the `timeout(10 s, pause.*.reached.notified())` at 12799, 12800, 12813 and 12814 with `QUERY_TIMEOUT` (30 s, the one write budget the receipt insert and `DOLT_COMMIT` spend); verify by reading the diff and running the two tests
  Evidence: done, with the derivation stated once above the block.
- [x] 7.2 m3#56: replace the `timeout(10 s, store.probe_logical_receipt(..))` at 12805, 12806, 12819 and 12820 with `QUERY_TIMEOUT` (30 s, `operation_receipt_matches` runs under it); verify by reading the diff and running the two tests
  Evidence: done.

## 8. packages/kuru-memory/src/store/migration_lifecycle_tests.rs

- [x] 8.1 m3#80: replace the `timeout(6 s, spawn_gated_open(options))` at 256 and 257 with a bound written out as the configured 1 s startup plus `SUPERVISOR_TRANSPORT_ALLOWANCE` (2 s) plus the `finish_owner` deadline `SUPERVISOR_REAP_ALLOWANCE` (13 s), derived from the options' own `startup_timeout_secs`; verify by reading the diff for the stated derivation and running the test
  Evidence: done; `refused_within` = `startup_timeout_secs` + `SUPERVISOR_TRANSPORT_ALLOWANCE` + `SUPERVISOR_REAP_ALLOWANCE` (16 s here), with the derivation at the site. `startup_timeout_secs` is unchanged.
- [x] 8.2 m3#75: delete the local `DEADLINE` (48, 10 s) and derive each use from `close_budget()` (the orphaned supervisor's stop of 11 s and reap of 13 s), with m3#77 (111), m3#78 (131, 177) and m3#79 (213, `Server::quiescence_at(.., DEADLINE)`) fixed by the same edit; verify by `rg DEADLINE` finding no constant left in the file and by running the lifecycle tests
  Evidence: done. The constant is gone. All six textual uses take a local `close_budget` bound to `crate::server::close_budget()`, whose comment names the close steps waited on (pool shutdown, the orphaned supervisor's stop and reap, the guard release).

## 9. packages/kuru-memory/src/store/migrations.rs (tests)

- [x] 9.1 m5#57: replace the `timeout(10 s, poll public_transcript_page)` at 4994 with `QUERY_TIMEOUT` (30 s, the statements run under `pool::within(QUERY_TIMEOUT)`); verify by reading the diff and running the test
  Evidence: done, with the derivation at the site: the owner commits the accepted resume within its write budget.

## 10. packages/kuru-memory/src/store/operational_gc_tests.rs

- [x] 10.1 m3#32: delete `TEST_DEADLINE` (line 4, 10 s) and derive each of its uses from the product value it waits on, with m3#36 to m3#40, m3#42 and m3#43 fixed by the same edit; verify by `rg TEST_DEADLINE` finding nothing in the file, by reading the diff for each use's derivation and by running the file's tests
  Evidence: done. The constant is gone, and a file comment states the derivation. Of its 24 uses, 22 now use `QUERY_TIMEOUT` directly: candidate operations, statements, writes, joins and session-end waits. Each waits on a step the product bounds by one statement or write budget, such as the session-end and branch-session waits of reconciliation and candidate transitions, or `write_deadline`. The 2 opens (the interrupted reopen reaching its recovery pause at 844, and the reopen at 854-855, which waits out the cancelled worker's reap and starts its own server) use `test_support::fixture_deadline(0, 1)`, the budget for one reopened lifecycle under the single-stall model.
- [x] 10.2 m3#33: replace the 80 ms absence windows at 99 (put), 184 (abandon) and 521 (new source pool) with an observable event inside the wait, a cfg(test) seam of a few lines being allowed and more recorded as deferred; verify by a test run
  Evidence: deferred, unchanged; each needs more than a seam of a few lines in an owned file. At 99 and 184 the put and the abandonment park in `resolve_uncertain`, then `await_session_end` (store.rs:7711). That is a free function over a pool, with no store or server handle, in `store.rs` product code outside the owned test module. An observer there needs a signature change or a new process-wide hook. At 521 the pool call parks on the admission gate in `Server::fence_pool` (server.rs:1195-1211), which is `server.rs` product code outside the owned files. The existing cfg(test) `pool_requests` log (server.rs:996-1001) records the call before the fence, so it cannot tell a parked call from one past a broken fence.
  Run: `mise run //packages/kuru-memory:test -- -- store:: <the server_lifecycle test names>`: 308 lib tests passed, 0 failed, 6 ignored, 1086.5 s.

## 11. packages/kuru-memory/src/store/recovery_tests.rs

- [x] 11.1 m3#16: replace the 80 ms `timeout(.., spawn_gated_open(options))` absence check at 1181 and 1182 with waiting for the opener's `MemoryOpenStage::WaitingForProjectOwnership` progress report, then asserting it is still pending; verify by a test run, and by temporarily letting the opener acquire (recorded, not committed) to see the test fail
  Evidence: done. Under the spawning guard, as `spawn_gated_open` holds it, the competing open runs as `MemoryStore::open_observed`. The test waits for `WaitingForProjectOwnership` (reported in `acquire_lock_reporting`, store.rs:8225, before its first sleep, only while another holder has the lock), selecting against the open finishing, which fails the test. That wait is bounded by the test's existing `completion_deadline` (`migration_observation_deadline`). The open is then dropped in its lock wait, as the old window dropped it. The temporary mutation was not run (see the note above). Run: see 10.2 (`cancelled_upgrade_call_retains_writer_through_accepted_ddl_boundaries` passed).
- [x] 11.2 m3#17: replace the 80 ms `timeout(.., store.reconcile())` absence check at 1255, and with it m3#18 (the 50 ms checks at 1299 and 1577), with an observable event that reconcile entered the session wait, then assert not finished; a cfg(test) seam of a few lines is allowed, more is recorded as deferred; verify by a test run
  Evidence: deferred, unchanged, for the same reason as 10.2. `reconcile` and `resolve_uncertain` park in `await_session_end` (store.rs:7711), product code outside the owned region that has no store or server handle for an observer.
- [x] 11.3 m3#1: delete `TEST_DEADLINE` (line 25, 10 s) and derive each of its uses from the product value it waits on (`QUERY_TIMEOUT`, `close_budget()` or `migration_observation_deadline`), with m3#6 (317), m3#7 (367, 369, 378, 380, including the Windows `child.wait(..)`), m3#8 (390, 398), m3#11 (551), m3#20, m3#21, m3#23 (the `await_flag` callers), m3#25 (1329) and m3#26 (3557) fixed by the same edit; verify by `rg TEST_DEADLINE` finding nothing in the file, by reading the diff for each use's derivation, by running the file's tests and by the Windows-target lint for the `cfg(windows)` uses
  Evidence: done. The constant is gone, and a file comment lists the derivations. Of its 39 uses:
  - 25 use `QUERY_TIMEOUT`: statements, writes, reconciliations, the `await_session_end` and `await_flag` callers, and the polls at 1329 and 3557.
  - 4 use `crate::server::SUPERVISOR_REAP_ALLOWANCE`: `take_quiescence` (317) and `stop_and_reap`'s child waits (367, 378 Windows) and reader (390). The killed creator's orphaned supervisor stops Dolt and exits within it, and a Windows child reports exit only once its Job's last process is gone. Comments are at both helpers.
  - 1 uses `crate::server::close_budget()`: the store close at 527.
  - 9 use `migration_observation_deadline(&options)`: the eight m3#22 sites, plus 551, where it is computed before the options move.
  `await_flag` keeps its signature, and every caller passes `QUERY_TIMEOUT`.
- [x] 11.4 m3#22: replace `TEST_DEADLINE` at 2508, 2610, 2745, 2759, 2765, 2904, 3016 and 3020 with `migration_observation_deadline(&options)` (startup plus `QUERY_TIMEOUT`, already used by the neighbouring waits); verify by reading the diff and running the migration tests
  Evidence: done at all eight. Run: see 10.2.

## 12. packages/kuru-memory/tests/fixtures/parent/windows.rs

- [x] 12.1 m4#114: replace the `timeout(5 s, ..)` at 119 with `kuru_memory::test_budgets::QUERY_TIMEOUT` (30 s, the pool acquire ceiling at server.rs:1119); cannot run here, so verify by `mise run //packages/kuru-memory:lint:windows`
  Evidence: done, with the derivation at the site (each poll needs a second pool connection while the transaction holds one). Not run here, since it is Windows-only. It compiles under the Windows-target lint (15.2).

## 13. packages/kuru-memory/tests/server_lifecycle.rs

- [x] 13.1 m4#88: replace the 150 ms `timeout(.., pending.as_mut())` absence check at 259 with observing the writable open at its lease wait (a new event, or a single poll once the cold reader holds the lease), then asserting it is still pending; a seam of a few lines in an owned file is allowed, else record as deferred; verify by a test run
  Evidence: deferred, unchanged. The writable open waits for the lifecycle lease inside its supervisor child (`Server::open` spawns `--internal-dolt-supervisor`, server.rs:630-660), so no in-process event marks that wait. The open's first suspension follows that spawn, so a single poll would be pending for an unrelated reason. A seam would sit in `server.rs` product code or the supervisor protocol, outside the owned files.
- [x] 13.2 m4#84: derive `options()`'s 20 s `timeout` at 40 from the default startup (30 s) as `OpenOptions::new(..).config.startup_timeout_secs`, which is public and exposes nothing new, for its 11 call sites (79, 114, 238, 250, 305, 336, 371, 488, 514, 539, 595); verify by reading the diff and running `server_lifecycle`
  Evidence: done, through `kuru_memory::OpenOptions` (its `new`, `config` and `startup_timeout_secs` are `pub`). Run: `server_lifecycle`, 11 passed and 0 failed in 28.4 s (only the env-gated helper was filtered).
- [x] 13.3 m4#90: replace the finite `sleep(60 s)` keep-alive at 542 with a peer that blocks until released (a read on a never-written stdin), so the helper stays alive until the test kills it; verify by running the test and reading that no sleep literal remains
  Evidence: done. The helper blocks in `spawn_blocking(|| stdin.read_to_end(..))`. The test now spawns it with a piped stdin, holds the write end unwritten, and drops it only after `kill()` and `wait()`. `parent_sigkill_closes_lifetime_pipe_and_allows_a_new_owner` passed. No sleep literal remains in the helper.
- [x] 13.4 m4#93: replace the 5 s deadline at 652 and 657 for the supervisor to exit after SIGTERM with `test_budgets::SUPERVISOR_REAP_ALLOWANCE` (13 s); verify by reading the diff and running the test
  Evidence: done, with the derivation at the site. The cleanup's own `from_secs(13)` a few lines later is a remainder row and is unchanged. `supervisor_sigterm_reaps_and_exits_while_the_parent_pipe_is_open` passed.
- [x] 13.5 m4#87: replace the `timeout(10 s, SELECT SLEEP(6))` at 92 and 93 with the statement's own product budget; keep the 6 s SQL `SLEEP` stimulus (it must exceed Dolt's 5 s timer); verify by reading the diff and running the test
  Evidence: done, as the lead ruled. The bound is `test_budgets::QUERY_TIMEOUT` (the pool acquire ceiling the statement first spends) plus the deliberate sleep, now named `SLEEP` (6 s) beside the unchanged `SELECT SLEEP(6)` text, with a comment to keep the two equal: 36 s. The comment that named "our 20-second budget" now names the configured startup budget, and the failure message names the derived bound. `configured_startup_budget_is_not_preempted_by_a_shorter_query_timer` passed.

## 14. packages/kuru-memory/tests/windows_lifecycle.rs

- [x] 14.1 m4#103: replace `child.wait(Duration::from_secs(15))` at 396 and 785 with `test_budgets::close_budget()` (32 s, the creator's normal real-Dolt close); read 556 and 626 from the code and leave them unchanged with that reason if confirmed; cannot run here, so verify by `mise run //packages/kuru-memory:lint:windows`
  Evidence: done at 396 and 785, with the derivation at each site. 556 and 626 are unchanged, as confirmed from code. At 556 the `marker-before` child writes its failure frame through `report_marker_startup` after the open has returned its error (test_support/windows.rs:75-82), so no engine close follows the frame. At 626 the wait follows `terminate()` of the retained handle or owned Job. Not run here, since it is Windows-only. It compiles under the Windows-target lint (15.2).

## 15. Checks

- [ ] 15.1 Run `mise run //packages/kuru-memory:test` and the narrower filters used while iterating, and record pass counts and any failure with its cause; state that local runs do not reproduce a loaded CI runner
  Evidence: pending.
- [ ] 15.2 Run `mise run //packages/kuru-memory:lint`, `mise run //packages/kuru-memory:lint:windows`, `mise run format:check` and `mise run typecheck` and record each exit code
  Evidence: pending.
- [ ] 15.3 Verify the diff touches only the owned files and this change's openspec directory (and not `mise.lock`, nor `src/test_support*`), and that no new `from_secs(` or `from_millis(` literal lacks a derivation, by reading the branch's commits and searching the diff for added literals
  Evidence: pending.
- [ ] 15.4 Validate with `mise run cospec -- validate memory-derived-waits --strict`, confirm the apply gate is clear, and archive before the final commit, confirming the archive exists
  Evidence: pending.
- [ ] 15.5 CI evidence: the native test jobs on Linux, macOS and Windows run the changed memory tests to green, including the Windows-only sites; awaits CI
  Evidence: awaits CI.
- [ ] 15.6 CI evidence: the combined coverage run holds the 90% workspace line gate with the changed tests; awaits CI
  Evidence: awaits CI.
