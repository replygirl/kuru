# Proposal

## Why

`packages/kuru-platform/tests/windows_process.rs` bounds 72 waits with `LIMIT`, a bare 8 s literal with no derivation, and two of its outer waits (`LIMIT * 2`, `LIMIT * 4`) use multipliers that match no series of waits. The process fixture parks its children on a flat 30 s sleep (and a 30 s process-wide guard) that the parent's own waits can outlive or only just outlast. No flat wait may decide an outcome: each wait ends on its event under a budget derived from a named source, and a fixture child that the parent terminates is parked until terminated, with a self-timeout derived from its parent's budget that only reaps an orphan.

## What Changes

Only `tests/windows_process.rs` and `tests/fixtures/process.rs` change, with no product code. Both sites were re-baselined on origin/main 94cf53c5: `packages/kuru-platform` is identical to the inventory base 327f817c (empty diff), `LIMIT` is at `windows_process.rs:20` with 72 uses, and the fixture sleeps are at `tests/fixtures/process.rs:118,156,165,217,255,313`.

### p1#31: derive `LIMIT` once, at its definition

Inputs found for the derivation (measured from source on this base):

| Input | Where | Value | Use |
| --- | --- | --- | --- |
| Platform-owned native bound | `src/windows/pipe.rs:742` (`stdio`: `accept_connection(Duration::from_secs(5))`), reached by every `Stdio::Pipe` spawn | 5 s | The only wall-clock bound the platform owns that these tests reach. Input: the native wait. |
| Fixture child's own bounds | `tests/fixtures/process.rs:235,252,258,272` (connect), `267` (close), `293` (`child.wait`), `306` (accept) | 5 s each | The parent must outlive them so the child's own error is observed first, as `engine_warm_up_bound` covers `provision`'s errors. Coupling, not a change (p1#53 is outside this change). |
| Child-start allowance | `packages/kuru-memory/src/test_support.rs:546` `CHILD_START_MARGIN` ("creating a fixture child process and its runtime before the product's own startup clock begins inside it") | 5 s | Stated allowance; `cfg(test)` and `pub(crate)` there, so restated here with the citation. |
| Consumer budgets for the same calls | `kuru-connectors/src/process.rs:44-49` (`stop`: terminate, then `wait(5 s)`), `tools.rs:2378-2379` (`close(5 s)`), `rpc.rs:50` (`CLEANUP` 5 s); `kuru-delivery/src/update_budget.rs:9` (`STARTUP` 10 s: connection, frames in flight, each pipe close) and `:15` (`CLEANUP` 10 s) | 5 s and 10 s | Cross-check only. Both crates depend on `kuru-platform`, so neither can be imported. |

No platform bound applies to these groups, because the API is caller-bounded and the test is the caller: `Pipe::close` (`pipe.rs:289`), `Pipe::accept_connection` (`pipe.rs:303`, via `PrivateServiceListener::accept` `:575` and `PrivateListener::accept` `:615-618`), `pipe::connect` (`pipe.rs:700`), `NativeChild::wait` (`process.rs:481`) and `wait_process_handle` (`process.rs:961`) all take their `timeout` from the caller, and `NativeChild::terminate` (`process.rs:501`) is an unbounded request (`TerminateJobObject` or `TerminateProcess`) proved by a later `wait`. Not inputs: `POLL_INTERVAL` 5 ms (`pipe.rs:46`) and the 10 ms polls (`process.rs:494`, `:975`) are cadence; `browser.rs:31` and `:199` (3 s) are not reached by this file.

Derivation, written as doc comments on the constants at `windows_process.rs:20` (`NATIVE_BOUND`, `CHILD_START`, `LIMIT = NATIVE_BOUND.saturating_add(CHILD_START)`): `LIMIT` = the native bound (5 s, `pipe.rs:742`, equal to the fixture's own bounds and to the consumers' 5 s) plus the child-start allowance (5 s, restated from `CHILD_START_MARGIN`, `test_support.rs:546`) = 10 s, equal to `kuru-delivery`'s `STARTUP` and `CLEANUP`. This derives 10 s where the literal was 8 s; the increase comes from the sources, not from runner speed, and applies only to the failure path of a wait. By class of wait it reads:

- Operations the platform bounds through its caller (`child.wait`, `accept`, `connect`, `close`, including the in-process service-pipe test): the native bound plus the child-start allowance where a child is started first.
- Readiness and handshake reads from a just-started child (`ready`, the starter and tree readiness lines, the hello read): pure fixture child work. No product bound applies, so `LIMIT` is the fixture's stated child-start-plus-work allowance.
- Polls and reads of fixture work (`unlocked`, `root_exited`, receipt polls, read-to-EOF after exit, the 8 MiB duplex join): pure fixture work with no product bound; same allowance.

The three outer `recv_timeout` waits are series bounds, not multiples. Counting each `LIMIT`-bounded call as written inside the spawned thread:

| Site | Thread series | `LIMIT` calls | `SHORT` | Now |
| --- | --- | --- | --- | --- |
| `windows_process.rs:830` | accept (793), hello read (795), close (810), close (812) | 4 | 2 (804, 808) | `LIMIT * 2` |
| `windows_process.rs:974` | accept (939), readiness (942), close (958), remainder read (962), close (967), `child.wait` (968) | 6 | 1 (951) | `LIMIT * 4` |
| `windows_process.rs:1018` | first connect (998) | 1 | 2 (989, 999) | `LIMIT` |

Neither multiplier matches its series, and the third equals its inner bound. Each becomes the series plus one `LIMIT` for what no wait inside the thread bounds (child creation, runtime construction and shutdown): 5 `LIMIT` + 2 `SHORT`, 7 `LIMIT` + 1 `SHORT`, 2 `LIMIT` + 2 `SHORT`. One small helper, `series(limits, shorts)`, replaces exactly these three sites, which meets the three-site rule; each call site writes its count and names what the extra `LIMIT` covers.

### p1#50 with p1#49: park fixture children until terminated, under a derived orphan guard

The six `tokio::time::sleep(Duration::from_secs(30))` (`idle` 118, `capture-stall` 156, `capture-flood` 165, `owner` and `owner-startup` 217, `rendezvous-stall` 255, `trusted-owner` 313) become `std::future::pending::<()>().await`, so only termination or the fixture's self-timeout ends them.

The process-wide self-timeout at `tests/fixtures/process.rs:9` (p1#49, `tokio::time::timeout(Duration::from_secs(30), run())`) is kept, not dropped (review ruling): `owner`, `owner-startup` and `trusted-owner` are launched as `Lifetime::TrustedSupervisor`, whose drop closes the handle without terminating (`src/windows/process.rs:74-76`), so without it a test that panics before its explicit `terminate()` would leave that fixture parked for good. It is derived instead of flat, through argument plumbing the fixture's positional argv parsing already supports:

- The parent passes the fixture's self-timeout, in milliseconds, as the argument after the mode's fixed arguments (`idle` index 1, `rendezvous-stall`, `rendezvous-partial-frame` and `trusted-owner` 2, `owner` and `owner-startup` 4), and the fixture uses it as is, so the derivation lives only next to `LIMIT`.
- That self-timeout, formatted by `budget_arg`, is the parent's budget on the fixture, a series of its waits from the spawn until it no longer needs the fixture running, plus one `LIMIT` for the parent's unbounded steps after its last counted wait (a thread join, a synchronous lock check, `try_wait`) and the gap between the parent's clock and the fixture's, which starts only once the fixture runs. So the parent's waits decide first and the guard only reaps an orphan.

The budget of each budgeted launch in `tests/windows_process.rs` is the span over which its parent needs it running:

| Launch | Parent's waits the fixture must outlast | Budget |
| --- | --- | --- |
| `idle`, `timeout_retains_tree_ownership_and_termination_is_explicit` | readiness; the `wait(SHORT)` that must time out | `series(1, 1)` |
| `idle` (`unrelated`), `owner_death_closes_its_job_...` and `owner_loss_at_child_startup_...` | own readiness; the owner's creation (no wait bounds it, one `LIMIT`); owner readiness; owner reap; the lock's release before `unrelated.try_wait()` | `series(5, 0)` |
| `idle` (`expected`), `wrong_peer_and_nonlocal_names_...` | readiness; the test's connect; the rejected accept; four rejected connects and the accept that must time out | `series(3, 5)` |
| `idle`, `diagnostic_sampling_...` and `process_object_retained_...` | readiness only | `LIMIT` |
| `owner`, `owner-startup`, `trusted-owner` | readiness only | `LIMIT` |
| `rendezvous-stall` | the outer `recv_timeout` it outlasts (`try_wait` after it must see the peer running) | `series(5, 2)`, the same binding as that wait |
| `rendezvous-partial-frame` | the outer `recv_timeout` its thread runs under; the peer ends on the EOF of the thread's close, inside that series | `series(7, 1)`, the same binding as that wait |

`series` is the helper that replaces the three outer waits; it also expresses these budgets because each is a series of the parent's waits, where a raw `LIMIT * n` would be the multiplier form ruled out above. That reuse is named for review.

Not budgeted, recorded with the reason: these launches keep the existing 30 s backstop as the guard's fallback, not derived here. `capture-stall` and `capture-flood` cannot take the argument within this change: their parent is `tests/windows_commands.rs` (`:333`, `:380`), outside its owned files; their parent's `capture_native_output` terminates and reaps them on timeout and overflow, the only outcomes those modes produce, within 16 s. The `console-owner` fixture's `idle` child: its parent is the fixture itself, whose 5 s bounds (p1#53) are outside this change; it ends on `interrupt()` and dies with that parent's `OwnedJob`. The `invalid_creation_intent_fails_before_running_a_fixture` launches are rejected before a fixture runs. `startup` polls for a release file its test never writes, so only the close of the `owner-startup` fixture's `OwnedJob` at that owner's termination ends it. `rendezvous` ends when its parent (the test, or `trusted-owner` at its termination) drops or closes the pipe, and `duplex` on its parent's payload, each about 20 s from its start at most against the 30 s fallback. The remaining modes end on their own work or on input their parent supplies.

Left alone, as constraints: the fixture's own 5 s bounds (p1#53, the coupling above) and its 20 s release deadlines at `process.rs:129` and `:179` (p1#51). Those deadlines already sit below the parent's worst serial path before it writes the release file (three `LIMIT` waits, 24 s at 8 s, 30 s at 10 s), and the derived `LIMIT` widens that gap on failure paths only.

## Impact

Test code only: `packages/kuru-platform/tests/windows_process.rs` (Windows-only, `cfg(all(windows, feature = "test-support"))`), `packages/kuru-platform/tests/fixtures/process.rs` (a no-op `main` off Windows, so host tests do not exercise the changed code) and this change's directory. No product code, dependency, workflow, documentation or coverage-threshold change; no retry; `startup_timeout_secs` untouched. The healthy path is unchanged: every changed bound is a failure-path deadline or a parked fixture that its parent terminates. The parked modes gain one optional trailing argument, which `tests/windows_commands.rs` and the fixture's own nested launches do not pass, so their 30 s backstop is unchanged. Behavior evidence is native only: CI's `native-platform` job (x86_64 `:coverage`, aarch64 `:test`) and the Windows native-tests partitions; this host runs the static Windows checks. Coordination with #207 (`fix/unix-spawn-pipe-inheritance`): it edits `src/unix.rs`, `src/unix/snapshot.rs`, `tests/unix_process_group.rs`, `packages/kuru-platform/Cargo.toml`, `AGENTS.md`, `docs/development.md` and the `native-platform` spec; this change touches none of them, so the two merge in either order.
