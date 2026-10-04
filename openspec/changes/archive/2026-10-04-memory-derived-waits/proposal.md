# Proposal

## Why

The test-wait inventory (`tmp/roadmap/test-wait-inventory-2026-10.md`, section 7)
found guessed bounds in kuru-memory test code that decide outcomes. The worst are
undercuts: a test bound shorter than the product budget the awaited operation may
legitimately take (`QUERY_TIMEOUT` 30 s, `close_budget()` 32 s,
`maintenance_deadline()` 32 s, the Dolt stop of 11 to 13 s). A slow but correct run
fails them, as in the runner failure that PR #181 diagnosed for the runtime tests.
This change fixes exactly the 40 priority-subset fix points in kuru-memory (7
absence or race rows, 33 bounds). A wait must end on the event it waits for, under
a budget derived from a product or vendor value with the derivation written at the
site; it is never lengthened because a runner is slow.

## What Changes

Each fix point takes one of two shapes: derive the bound from the product value it
undercuts (derivation in a comment or doc comment at the constant or site), or
delete the inner bound where an enclosing
`FixtureDeadline::start(fixture_deadline(..))` already bounds the same fixture and
its expiry reports the hang as well. A derived replacement is at least the product
budget named; where an operation encloses several product-bounded steps, the budget
of the step it waits on or the enclosing `fixture_deadline` is used. Where the
inventory's fix shape and the traced product budget disagree, the traced budget
wins. Absence and race rows end on an observable event instead of a fixed window,
using the named barrier or a cfg(test) seam of a few lines in an owned file; a row
that needs more is recorded as deferred with its reason. Each task records, per
site, done (with its shape) or not done (with the reason found in code).

Re-baseline (measured on origin/main 449dca9e, base of this branch): `git diff
327f817c origin/main` touches only `packages/kuru-runtime`, the archived
`runtime-post-cancel-joins` change and its openspec files; nothing under
`packages/kuru-memory`, `apps` or `docs`. Every cited line was probed on this base
and holds the cited code, and every product reference (`QUERY_TIMEOUT` store.rs:66,
`CLOSE_GRACE`/`KILL_GRACE` server.rs:65-66, `SUPERVISOR_REAP_ALLOWANCE` server.rs:73,
`close_budget()` server.rs:171, `maintenance_deadline()` service.rs:2664,
`OPERATION_TIMEOUT` rpc.rs:38, `HANDLER_BUDGET` rpc.rs:718, `VERSION_TIMEOUT`
provision.rs:28, `LOCK_TIMEOUT` provision.rs:27) holds too. Result: 0 sites moved,
0 sites already fixed. Open PR #208 adds 25 lines to service.rs product code
(`spawn_service`, hunks at 1372-1441), so if it merges first every service.rs line
below 1441 shifts by that amount; re-grep before editing.

Non-test edits (visibility only, no value or behaviour change):

- Exception (b): one new file, `packages/kuru-memory/src/test_budgets.rs`,
  declared in `lib.rs` as
  `#[cfg(any(test, feature = "test-support"))] pub mod test_budgets;`. It
  forwards, unchanged and each with a doc comment naming its product source,
  only what a site here or in the kuru-tui PR derives from: `OPERATION_TIMEOUT`
  (`service::rpc`, widened from private to `pub(crate)`), `QUERY_TIMEOUT`
  (`store`), `close_budget()` and `SUPERVISOR_REAP_ALLOWANCE` (`server`, both
  already `pub(crate)`). It is the first implementation commit, on its own
  (`test(memory): expose product budgets to test-support consumers`), so the
  kuru-tui PR can cherry-pick it. `test_support*` is not touched.
- Exception (a): an existing product constant or fn may be widened to
  `pub(crate)` for in-crate tests, for example `HANDLER_BUDGET` (rpc.rs:718).

### Sites

Rows are the inventory ids. "Lines" are on the base above. "Derived from" is the
intended shape; the implementer confirms it from code and records any difference
in the matching task.

| Id | File and lines | Bound now | Derived from |
| --- | --- | --- | --- |
| m2#4 | `facade.rs` 3308, 3775, 4018, 4214, 4651, 4855, 4962, 5102, 5470, 5649, 5699, 5855, 5937, 6101, 6232, 6350, 6428, 6527, 6612 | `reap_within` 10 s of `served.retire/restart/reap` | `crate::server::close_budget()` (32 s) passed by the caller; `ServedOwner` signatures unchanged |
| m2#6 | `facade.rs` 3398, 3459, 3539, 3615, 3914, 4164, 4303, 4379, 4567, 4806, 4925, 5043, 5294, 5559, 5792, 6018 | `timeout` 10 s around a sibling poll | `QUERY_TIMEOUT` (30 s, write budget), or delete for an enclosing `FixtureDeadline` |
| m2#7 | `facade.rs` 5586, 5837, 6043 | `timeout` 10 s around a retry loop | `maintenance_deadline(&options)` (32 s) |
| m2#15a | `facade.rs` 6673 | 250 ms absence window | the owner's first refused (`acquired: false`) reply to the waiter, via a seam of a few lines, else deferred |
| m2#10 | `facade.rs` 6733, 6949 | `permit_within` 20 s | `Some(maintenance_deadline(&options))` (32 s) |
| m2#13 | `facade.rs` 6813 | `timeout` 5 s on an independent read | `OPERATION_TIMEOUT` (35 s, the call's reply deadline, above `QUERY_TIMEOUT`), or delete for the `FixtureDeadline` |
| m5#22 | `provision/tests.rs` 1039, 1127, 1184, 1792 | `cache_lock(.., 10 s)` | `LOCK_TIMEOUT` (180 s, provision's own lock budget, above `VERSION_TIMEOUT` 15 s that the lease holder may run) |
| m5#24 | `provision/tests.rs` 1090, 1093 | 5 s wait for the `started` marker | `VERSION_TIMEOUT` (15 s) |
| m4#56s | `server_tests.rs` 241 | 30 ms quiescence absence | the releaser task's own signal after `observe_dolt`'s error path returns, then assert the lease is held |
| m4#60 | `server_tests.rs` 732, 850 | `timeout` 10 s on the supervisor | `SUPERVISOR_REAP_ALLOWANCE` (13 s, above stop 11 s plus 1 s drain) |
| m1#5 | `service.rs` 1532, 1543 | 10 s `wait_for_exit` | `close_budget()` (32 s) |
| m1#36 | `service.rs` 5482, 5483 | 5 s on the busy-owner purge | `maintenance_deadline(&options)` (32 s) |
| m1#37 | `service.rs` 5509, 5510, 5544 | 20 s permit and `permit_within` | `maintenance_deadline(&options)` (32 s) |
| m1#51 | `service.rs` 6677, 7293, 7335, 7410, 7546, 7571 | `timeout` 10 s on a join | `QUERY_TIMEOUT` (30 s) |
| m1#53 | `service.rs` 6703 | `timeout` 10 s on `owner.close()` | `close_budget()` (32 s) |
| m1#54 | `service.rs` 6736 (16 call sites) | `Served::finish` 20 s | `QUERY_TIMEOUT` (30 s, one request round trip) |
| m1#56 | `service.rs` 6855 | `SettlementFixture::close` 20 s | `close_budget()` (32 s) |
| m1#57 | `service.rs` 6890 (17 call sites) | `next_wait_event` 10 s | `QUERY_TIMEOUT` (settlement wait budget, rpc.rs 897-908) |
| m1#59 | `service.rs` 6925, 6964, 7005, 7078, 7097, 7168, 7359, 7406, 7435, 7468, 7526 | `entered.notified()` 10 s | `QUERY_TIMEOUT` (30 s) |
| m1#64 | `service.rs` 7792, 8261, 8323 | `timeout` 10 s on a poll loop | `QUERY_TIMEOUT` at 7792; `HANDLER_BUDGET` (30 s) at 8261 and 8323 |
| m2#29 | `service/rpc.rs` 1006, 1023, 1179 (cfg(test) pauses) | `release.notified()` 10 s | `OPERATION_TIMEOUT` (35 s, same file) |
| m4#79 | `spawn_gate.rs` 490 | 200 ms absence window | the taker observed queued (`GATE.lock.try_read()` fails), as the sibling tests' `queued_seen` |
| m3#55 | `store.rs` 12799, 12800, 12813, 12814 | `reached.notified()` 10 s | `QUERY_TIMEOUT` (30 s, write budget) |
| m3#56 | `store.rs` 12805, 12806, 12819, 12820 | `probe_logical_receipt` 10 s | `QUERY_TIMEOUT` (30 s) |
| m3#80 | `store/migration_lifecycle_tests.rs` 256, 257 | `timeout` 6 s around the gated open | configured 1 s startup plus `SUPERVISOR_TRANSPORT_ALLOWANCE` 2 s plus `SUPERVISOR_REAP_ALLOWANCE` 13 s, written out |
| m3#75 | `store/migration_lifecycle_tests.rs` 48 (uses 111, 131, 177, 213) | `DEADLINE` 10 s | `close_budget()` per use (stop 11 s, reap 13 s); constant deleted |
| m5#57 | `store/migrations.rs` 4994 | `timeout` 10 s on a poll | `QUERY_TIMEOUT` (30 s) |
| m3#32 | `store/operational_gc_tests.rs` 4 (25 uses) | `TEST_DEADLINE` 10 s | per use: `QUERY_TIMEOUT` for candidate ops, statements and joins, the startup budget for opens; constant deleted |
| m3#33 | `store/operational_gc_tests.rs` 99, 184, 521 | 80 ms absence windows | an observable event inside the wait (reconcile parked on the live session, pool call parked on the fence), else deferred |
| m3#16 | `store/recovery_tests.rs` 1181, 1182 | 80 ms absence window | the opener's `MemoryOpenStage::WaitingForProjectOwnership` report, then assert still pending |
| m3#17 | `store/recovery_tests.rs` 1255 (with 1299, 1577) | 80 ms and 50 ms absence windows | an event that reconcile entered the session wait, else deferred |
| m3#1 | `store/recovery_tests.rs` 25 (40 uses) | `TEST_DEADLINE` 10 s | per use: `QUERY_TIMEOUT`, `close_budget()` or `migration_observation_deadline`; constant deleted |
| m3#22 | `store/recovery_tests.rs` 2508, 2610, 2745, 2759, 2765, 2904, 3016, 3020 | `TEST_DEADLINE` 10 s | `migration_observation_deadline(&options)` (startup plus `QUERY_TIMEOUT`) |
| m4#114 | `tests/fixtures/parent/windows.rs` 119 | `timeout` 5 s | `test_budgets::QUERY_TIMEOUT` (30 s, the pool acquire ceiling) |
| m4#88 | `tests/server_lifecycle.rs` 259 | 150 ms absence window | the open observed at its lease wait, or one poll after the reader holds the lease |
| m4#84 | `tests/server_lifecycle.rs` 40 | `options()` timeout 20 s | the default startup (30 s) via `OpenOptions::new(..).config.startup_timeout_secs` |
| m4#90 | `tests/server_lifecycle.rs` 542 | `sleep(60 s)` keep-alive | a peer that blocks until released (read on a never-written stdin) |
| m4#93 | `tests/server_lifecycle.rs` 652, 657 | 5 s for the supervisor to exit | `test_budgets::SUPERVISOR_REAP_ALLOWANCE` (13 s) |
| m4#87 | `tests/server_lifecycle.rs` 92, 93 | `timeout` 10 s around `SELECT SLEEP(6)` | `test_budgets::QUERY_TIMEOUT` (the pool acquire ceiling) plus the deliberate `SLEEP` |
| m4#103 | `tests/windows_lifecycle.rs` 396, 556, 626, 785 | `child.wait(15 s)` | `test_budgets::close_budget()` (32 s). 556 and 626 are recorded in the inventory as not qualifying (frame written after cleanup, `TerminateProcess` exits at once); the implementer decides from code |

Dependents fixed by the same edit as their row: m2#30 and m2#31 (rpc.rs 1023 and
1179), m3#36 to m3#40, m3#42 and m3#43 (the `TEST_DEADLINE` uses in
`operational_gc_tests.rs`), m3#6, m3#7, m3#8, m3#11, m3#20, m3#21, m3#23, m3#25 and
m3#26 (the `TEST_DEADLINE` uses in `recovery_tests.rs`), m3#18 (with m3#17) and
m3#77 to m3#79 (the `DEADLINE` uses in `migration_lifecycle_tests.rs`).

Integration-test sites (ruled after the scope stage): the five sites in separate
crates that see only public items (m4#84, m4#87, m4#93 in
`tests/server_lifecycle.rs`, m4#103 in `tests/windows_lifecycle.rs`, m4#114 in
`tests/fixtures/parent/windows.rs`, all built with the `test-support` feature)
derive from `kuru_memory::test_budgets::*` (exception (b)), and m4#84 from the
public `OpenOptions` config `startup_timeout_secs`. Deriving from an unrelated
public constant that merely exceeds the value (for example `OPERATION_TIMEOUT`
standing in for `close_budget()`) is not a derivation and is not used.

Not changed: product behaviour and constant values, `startup_timeout_secs`, any
remainder row (a remainder fix point on a line being edited may go with it,
otherwise it stays, for example the 5 s literal at `service.rs` 6663), anything
under `packages/kuru-memory/src/test_support*`, `ServedOwner::retire/restart/reap`
signatures, any retry, any new crate or dependency, and any new helper unless it
replaces three or more sites. No assertion after a wait changes meaning.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- Owned files only: the new `packages/kuru-memory/src/test_budgets.rs`, its one
  declaration line in `src/lib.rs`, `packages/kuru-memory/src/facade.rs`, `provision/tests.rs`,
  `server_tests.rs`, `service.rs` (test module, lines 3411 and above on 327f817c,
  plus its cfg(test) fixture code), `service/rpc.rs` (cfg(test) code, plus the
  visibility-only widenings above), `spawn_gate.rs` (tests), `store.rs` (test module),
  `store/migration_lifecycle_tests.rs`, `store/migrations.rs` (tests),
  `store/operational_gc_tests.rs`, `store/recovery_tests.rs`,
  `tests/fixtures/parent/windows.rs`, `tests/server_lifecycle.rs`,
  `tests/windows_lifecycle.rs`.
- Test only: no product behaviour, constant value or `startup_timeout_secs`
  change. Windows-only sites cannot run here and must compile under
  `mise run //packages/kuru-memory:lint:windows`.
- CI time: no wait gets shorter by design; absence rows stop depending on a
  window, and bounds that were below a product budget now equal or exceed it.
- Coordination: open PR #208 (`fix/instrumented-child-outlives-test`) edits
  `service.rs` product code at `spawn_service` (1372-1441), above this change's
  lowest `service.rs` site (1532), so there is no textual overlap but the line
  numbers shift. The `fix/endpoint-record-replaced-name` branch currently holds only
  its cospec scope commit and no kuru-memory code; it will touch the endpoint-record
  read path in `service.rs` product code, which this change does not touch. Older
  feature stacks (`feat/explicit-legacy-memory-import`,
  `feat/p20-restorable-memory-backup`, `feat/p23-doctor-integration`) edit
  `facade.rs`, `store.rs`, `store/recovery_tests.rs` and `store/migrations.rs`; expect
  rebase work there, in separate hunks.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
