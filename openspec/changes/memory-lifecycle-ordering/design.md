# Design

## Context

### Status statement

**Fault proven by code and injected test; natural trigger observed on CI (macOS 26, 3 vCPU), not reproduced on the 14-core local host.**

This applies to H1. H2 is proven on the local host: the close-time panics reproduce there and are attributed to a directory removed before Dolt exited.

### Root cause H1: an ungated candidate delete

Upstream Dolt v2.3.5 (`dolt_branch.go:131-137`, `:253-259`, `:309-359`) runs one check, `validateBranchNotActiveInAnySession`, for both a checked `DOLT_BRANCH` delete and a rename, before any merge check.

- It fails with 1105 `unsafe to delete or rename branches in use in other sessions; use --force to force the change` while any other session has that branch checked out. Idle pooled sessions count.
- A session is removed by GMS `SessionManager.RemoveConn`, which is called from the vitess per-connection goroutine's `ConnectionClosed` once its read loop exits. That is asynchronous from the client's close.
- `RemoveConn` removes the connection from the processlist under the same lock. So the connection disappearing from `information_schema.processlist` is a valid server-observed signal.

On c986f4ff (`packages/kuru-memory/src/store.rs`):

- `transition_candidate_with_retirement_deadline` (1175) runs `fence_pool`, `retire_pool`, then `await_branch_sessions_end(.., retirement_deadline)` (1194) before the rename.
- `delete_candidate_ref` (1250) runs `retire_pool` (1265), then an owned connection, then `DOLT_BRANCH('-d'|'-D')` (1281), with no session wait.
  - Just before it, `candidate_branch_is_clean` (1094) opens a fresh branch pool, runs one query and retires it.
  - The abandon path's `confirm_no_live_candidate_session` (1301) does a checked `-m b b` self-rename that hits the same check.
- `Server::retire_pool` (server.rs:817) awaits only the client-side `Pool::close()`.
- Every catalogued 1105 shows `stage=cleanup`, never `stage=branch_rename`.
- `candidate_branch_rename_reason` (759) labels the message `branch_in_use` only at `BranchRename`, which is why the cleanup records read `reason=other`.

### Evidence

**Deterministic reproduction (local, on the rebased measurement commit).**

- The ignored measurement `store::lifecycle_measurement_tests::measure_lingering_session_consequence_for_delete_and_rename` detaches one real server session on `kuru/<promoting>` that closes itself after 20, 100 or 1000 ms.
- Control (session closed first), 5/5: the delete gets past the in-use check and fails `not fully merged`.
- Every linger, 5/5 each, 15/15 in all: `delete_candidate_ref` fails in 3-4 ms with `candidate_owner stage=cleanup class=database sqlstate=HY000 vendor=1105 reason=other`. That is byte-identical to catalogue items 5 and 12.
- In the same runs, `transition_candidate` waits the linger out and succeeds (62-65, 140-153 and 1042-1055 ms).

**Natural trigger.**

- PR #115's final3 CI log (macOS 26, 3 vCPU) shows the canary printing `active_sessions=2` immediately after `retire_pool`.
- On the 14-core local host:
  - 0 of 600 idle and 0 of 600 saturated linger samples saw a session at the first poll.
  - 0 of 1,200 time-zero checked deletes would have failed.
  - 0 of 2,400 product abandons failed.
  - The five CI-failing memory tests passed 1,000/1,000 (200 iterations).
  - The canary saw 0 sessions after retirement in 200/200.
- The server-side cause of the linger was not isolated.

**Supersedes dx-followons §16.** Its "log the SQL around candidate_owner cleanup and rerun the partitions until a hit" capture plan is superseded by the deterministic reproduction above. No draft capture branch is needed.

### Root cause H2: a directory removed before Dolt exits

The close-time panic signatures are `store.go:1762` `fatal error closing table persister`, and `file_manifest.go:556-558` / `:489` / `:578-584`.

- Every one requires the data directory or its manifest to be removed while Dolt is writing its final manifest.
- Dolt deliberately crashes on this (`SetCrashOnFatalError`). Upstream dolthub/dolt#10971 is open.

Before-baseline (local host, traced with `KURU_TEST_DOLT_LOG_DIR`):

| run | Dolt servers | close-time panics | directory removed before Dolt exited |
|---|---|---|---|
| kuru-memory full suite #1 | 778 | 1 | 4 |
| kuru-memory full suite #2 | 782 | 2 | 4 |
| kuru-runtime full suite | 337 | 2 | 5 |
| kuru-runtime loop: `dream::cancellation_tests::managed_lost_promotion_reply_keeps_report_and_publishes_only_typed_revision`, 100 iterations | 895 | 169 (148 signature a, 18 signature b, 3 bare) | 133; plus 132 unsuccessful supervisor exits |

The draft PR #117 CI capture found 26 panics in 7,002 dolt-live logs, about 9 per run.

**Class A (product).** `MemoryStore::open` can return `Err` after `Server::open*` succeeded without awaiting close. The sites on c986f4ff are:

- the `server.pool("main")?` calls at store.rs 1724, 1770 and 1803 (1803 accounts for the `.staging-*` hits)
- `migrations::version` (1878)
- `validate_supported`/`bail!` (1887-1890)
- `validate_inspection`/`validate_active` (1910-1914)
- `run_candidate_recovery_worker` and `usage_ledger::establish` (1938-1939)

On those paths the server is handed to `Owner::drop`'s background reaper and the test's root is deleted. Only the read-only refusal (1880) already awaits `close_installed_guard`.

**Class B (fixtures).** `packages/kuru-runtime/src/dream.rs` (~993-1114) drops its data `TempDir` right after the client-side `close()`. The service deliberately runs until its `SERVICE_IDLE_TIMEOUT` (30 s) idle grace.

### Windows `StagedActivation`

`provision.rs:792` declares `StagedActivation { source, probe, staging: PrivateTemp, lock: CacheLock }`.

- Publication (`finish_published`) and failure (`retain`) destructure it and decide stage ownership before releasing the lock.
- Cancellation drops the struct implicitly, so `PrivateTemp` falls to `tempfile::TempDir::drop`, which ignores a failed `remove_dir_all`.
- `files.rs` already has the checked, bounded Windows removal (`PrivateTemp::close` → `close_windows_private_stage`, bounded by `CLEANUP_RETRY_LIMIT`) that publication uses. The cancellation path does not use it.
- The test assertion `!stage_path.exists()` (now `provision/native_tests.rs:951`) sees the swallowed failure. That is item 6.

### Ruled out

- **Heavy load.** Item 12 recurred in a 92.63 s partition (PR #115 head 0c468e4e, job 108615291670).
- **Resource exhaustion.** Three instrumented macOS runs (#117, run 36290042959 attempts 1-3) showed about 4.5k of 30,720 open files, more than 99.9% of inodes free, about 88 GiB of disk free and maxproc headroom. The failing shards were indistinguishable from the passing ones.
- **dolthub/dolt#11796** (journal bootstrap race, fixed in v2.3.4) as the cause. Item 12 recurred on v2.3.4. The in-use check and its message are byte-identical in v2.3.3, v2.3.4 and v2.3.5.
- **A product directory move** as the cause of the close-time panics. Of the 196 affected servers, 0 had a `lease_move` or `lease_remove_tree` of their directory. The staging→active activation is gated by `Server::quiescence_at`.

## Goals / Non-Goals

**Goals:**

- Remove the H1 trigger: no candidate ref is deleted, renamed or self-rename-probed while the server still counts a session on it.
- Make `MemoryStore::open` returning `Err` imply that the engine is closed within the existing bounded-close contract.
- Make fixtures release a store directory only after its Dolt has been reaped, and make any violation fail the offending test.
- Disposition every catalogue entry.

**Non-Goals (OUT OF SCOPE, with reasons):**

- **The contract question.** `promote_checked` marks promotion only after cleanup succeeds, so a cleanup failure after a committed `DOLT_MERGE` makes the service reply `StorageFailed` and fences a client whose write landed. Changing what the client is told there is a contract change to the typed service faults and the fence. It needs its own change and review. This change removes the trigger only. The `branch_in_use` label extension is a diagnostic-record change and does not alter any reply.
- **Readiness deadlines** (items 13 and 14, and the deadline sub-items of 10 and 11). This is a separate family, with one local hit: a migration server ran 22.7 s against 5.2 s in the neighbouring iteration, and no 1105 or cleanup record was present. It is routed to the First-launch budget model item (phase2-handoff-2026-09-26.md, Phase 2 item 9).
- **The kuru-delivery bundle lock `WouldBlock`** (items 7a and 9). This is a different package and mechanism (flock held or inherited across concurrent tests or spawned children). It will be the second, small change after this one.
- **Upstream Dolt changes** (making #10971's close-time `Fatalf` non-fatal). Kuru must not remove a live directory in the first place.
- **Raising `RUST_TEST_THREADS`.** It stays 2 on every OS.
- **The Windows cancelled-activation teardown** (items 1 and 6). Split into its own change, branch `fix/memory-provision-stage-teardown` (see D6). Implementer decision pending maintainer confirmation.

## Decisions

**D1. One retirement step that ends at server-observed session end.**

- Every candidate ref delete, rename and exclusion probe is preceded by pool retirement plus `await_branch_sessions_end` on `kuru/<branch>`, the processlist predicate the rename already uses.
- Preferred placement: inside the retirement primitive, so that `Server::retire_pool`'s existing canary assertion (`active_after_close == 0`) becomes a guarantee rather than a race.
- If `Server` cannot perform the processlist observation without new authority, use a single store-level `retire_candidate_sessions(store, branch, deadline)` at every product site, and point the canary at that same product step with its assertion unchanged.
- Outcome: the fallback was taken. The step is the store-level `retire_branch_sessions(store, branch, deadline)`, used before every rename, delete and exclusion probe. The canary `candidate_pool_retirement_observes_exact_server_sessions_before_rename` now calls that step (with `QUERY_TIMEOUT`) instead of `Server::retire_pool`; every assertion line is byte-identical.
- Alternatives rejected:
  - Retrying the delete on 1105: forbidden, and it masks a live session.
  - `--force`/`-D` as force: skips the check and leaves other sessions on a missing ref (dolthub/dolt#9598, #6100).
  - `KILL CONNECTION`: also asynchronous; it only triggers the same teardown.
  - `dolt_branch_activity`: off by default and costly.

**D2. Existing budgets only.**

- The session wait uses `QUERY_TIMEOUT`, the rename's existing retirement deadline. It keeps a deadline parameter so a test can prove the never-ending-session failure without new product timing.
- Close uses `CLOSE_GRACE` and `SUPERVISOR_REAP_ALLOWANCE` (`close_budget()`).
- No constant is added or raised.

**D3. The fence is unchanged.** A session that outlives the deadline fails the step as `PoolRetirement`. The service still returns `StorageFailed` and the client still fences. A genuinely uncertain outcome still trips the fence.

**D4. H2-A: a bounded close on every post-start open error.**

- Wrap the post-start section so any `Err` first awaits `server.close_installed_guard()` (or `close_migration_worker` for migration workers) and attaches a close failure as context. This follows the existing store.rs:1755-1759 pattern.
- `Owner::drop`'s background reaper remains for cancellation only.

**D5. H2-B: explicit retirement plus a failing, record-based invariant.**

- Managed fixtures call a test-support helper, `await_managed_quiescence(&options)`: `retire_idle_service`, then `await_store_quiescence` on the project store and each remaining `.staging-*` sibling. `await_store_quiescence` waits for the store's lifecycle lease with `Server::quiescence_at` within `SUPERVISOR_REAP_ALLOWANCE` (a timeout is returned as an error, failing the fixture) and, while it holds the lease, writes a quiescence record. It runs before the data root is released.
- Records live in a process-local ledger (`test_support/engine_ledger.rs`). The ledger has two sources, both evidence this process observed itself: (1) every Dolt supervisor this process spawns is registered live when its `Owner` is built (server.rs, both platform arms) and leaves the ledger only when this process has reaped that supervisor (`finish_owner`, or the thread a dropped `Owner` hands its child to); that reap writes the store's record, so every store close path records (a dropped `Owner`'s reaper sends a report of the reap, and the ledger records it, as of the reap, on the next thread that reads it; a record never replaces a later one); (2) `await_store_quiescence` records while it holds the lease, for engines run by other processes (managed service owners, spawned `kuru` executables) and lease-only directories. A record is keyed by the store directory's native identity plus its birth time, so it follows renames. A native identity names a directory only while it exists (`FileIdentity` is authoritative only while its handle is open; Linux recycles a removed directory's inode at once), so releasing a fixture root forgets the records taken beneath it, and the birth time separates a store removed some other way (Linux stamps it at clock-tick granularity, far shorter than an engine run). The record snapshots the engine-written `server.log` and `endpoint.json`; any later engine start in any process publishes an endpoint and its stop rewrites the log, so a record made before it no longer matches.
- The fixture root returned by `test_support::tempdir()` gets a checked teardown (`test_support/fixture_dir.rs`). It uses lock files only to find stores (a directory holding `lifecycle.lock`; the `memory/<hash>` store of a `<hash>.service-owner.lock`; a directory holding `identity.json` whose native identity names an external `lifecycles/<identity>.lock`, the Windows lease, recognised on every platform; a template or an unopened copy carries `identity.json` without a lease and is not a store). A store fails the teardown if an in-process supervisor serving it is unreaped, if it has no record, or if its record is stale. The teardown then keeps the whole root and fails the test, naming the root, the test and each store with its reason. It never takes, probes or waits on a lock, so a descriptor duplicated into a sibling thread's child between `posix_spawn` and `exec` cannot make it fail; `fixture_dir::tests::a_duplicated_lease_descriptor_after_awaited_quiescence_does_not_fail` holds such a duplicate across the drop.
- While the thread is already panicking, it only keeps the directory. It does not panic, because a panic during unwinding would abort the process. The ledger mutex is read through poisoning for the same reason.
- Ledger and guard code runs only on threads that finish before their test function returns (CI round 2). Coverage writes a process's profile at exit; a thread still inside an instrumented function then can have its entry counter written without a later counter, which leaves a counter expression negative, and the partition's line export refuses it. The instrumented increments are atomic, so the ledger's critical section never addressed this; it keeps a teardown's verdict and its forgetting of records consistent. A fixture that must wait for a creator process releases or keeps its root with `test_support::release_after_creator_exit` on the calling thread. A dropped `Owner`'s reaper thread, which outlives its owner by design, runs no ledger code: under test-support it sends one report built from standard-library calls, and `engine_ledger::with` records pending reports first.
- A fixture that serves an in-process `ServiceOwner` on a task retires it on every exit path (CI round 3, attempt 2). The root, the options and a `test_support::ServedOwner` slot live outside the judged body; the body's `Result` is captured while the root lives and passed to `test_support::settle`, whose teardown is the success path's own retirement (`ServedOwner::retire`: maintenance permit, then the reap, with that fixture's bounds). A body error is printed to the captured output before the teardown starts and is returned in preference to a teardown failure. The slot empties once a task has ended, so a successor fixture never polls a finished owner twice, and owner tasks are no longer aborted on drop inside the body. The guard, the ledger and the product are unchanged: a teardown that cannot retire the owner still fails the test at the root's drop.
- The helper's reap bound is `server::SUPERVISOR_REAP_ALLOWANCE` itself (exposed `pub(crate)`), not a copied literal; the value is unchanged (13 s).
- Lock probes remain only where the process owns the engine or under the spawn gate: the kuru-memory tests that assert a lease is free right after an open error (`open_error_reap_tests`, `server_tests`) hold `spawn_gate::locking_async`, and the fixture guard itself has none. The earlier non-waiting flock probe and its spawn-race exposure in kuru-runtime and kuru-tui are removed, not accepted.
- Known spurious-failure edge, unreachable in current fixtures: an `Owner` dropped while live (the cancellation path) records its store only when its reaper thread observes the exit, polling every 20 ms. If the product moved that directory under the lifecycle lease inside that window, the record is snapshotted from the old path and a later teardown reads the moved store as stale. In-process moves go through `LifecycleLease::move_to`, where a re-key would belong if a fixture ever reaches this.
- The quiescence wait takes no spawn gate: it is a bounded waiting acquisition, not a probe, and nothing reads that lock's state afterwards.
- Detection limit (a missed violation, never a spurious failure): an engine run by another process that is still starting, before it publishes its endpoint, under a store that already has a current record, is not seen. Fixtures await quiescence after their child processes exit, so no current fixture reaches that state.

**D6. Windows: split out of this change.**

- The intended design (a checked `StageLease` teardown, stage before lock, a refused removal reported rather than swallowed by `TempDir::drop`, bounded only by the existing `CLEANUP_RETRY_LIMIT`) is drafted on branch `fix/memory-provision-stage-teardown`, unmerged and unverified on Windows.
- It is not in this change because, by static reading, it makes `cancelling_checked_activation_recovery_drops_stage_before_cache_lock` fail deterministically on Windows: the test's own `_parent` directory handle keeps `runtime` delete-pending under kuru-platform's legacy delete disposition, so the checked removal correctly retains the stage and `!stage_path.exists()` fails. The fixture handle needs a maintainer decision that does not weaken that assertion.
- Open in the draft: `close_published` releases the cache lock before the caller writes the retained-stage receipt, and `StageLease::drop` blocks the dropping thread for the bounded Windows recovery window on cancellation.
- Item 1's diagnostic assertion is re-examined from its CI log under that change.

**D7. Fixture rule: a single reconcile after a cancelled paused write orders on the owner's reply, not on a sibling's read (PR #125 residual 1).**

- Mechanism: lost-reply tests cancelled the paused client once a sibling could read the committed row, then asserted `reconcile() == Some(true)` from one call. The owner answers `ServiceCall::Outcome` (and the candidate, candidate-transition and ledger outcome queries) from its in-process receipt registry: a request's receipt is `Running` from registration until its handler returns (`RunningReceipt` dropped inside `respond`'s `processed` block), and a `Running` receipt is answered `InFlight` without a storage query. Dolt exposes the row to another session before `CALL DOLT_COMMIT` has replied to the owner, and the owner then still has the SQL `COMMIT` round trip to make, so the sibling's read and the receipt's settlement are unordered. A reconcile in that window gets `InFlight` and fails with "memory service write outcome remains uncertain" although the write committed. That report is truthful under the current contract; the test ordered on the wrong event.
- Rule: such a test awaits `ReplyPause::replied` (`ReplyBarrier::wait_replied`) before it cancels. The test-only paused exchange (`exchange_attached_with_id_paused`) reads the owner's reply frame while paused, raises `replied` once a frame has arrived, holds the frame (or a read error) and returns it only when `release` fires, as before. The owner writes the reply frame only after `processed` has completed, so its arrival strictly follows settlement. One reply deadline (`OPERATION_TIMEOUT`, 35 s) bounds the whole pause, including the frame read; before, the release wait had that budget and the read another. Cancellation is unchanged: `call_with_id` takes the stream at entry and restores it only after the exchange returns, so an abort anywhere in the pause leaves the attachment incomplete and the pending-mutation guard raises the fence. The sibling poll stays as the proof that the owner committed; every assertion is unchanged, and nothing polls `reconcile`.
- `ReplyBarrier` in kuru-runtime uses the same exchange. Its tests release the pause and await the call normally, or retire the owner before reconciling; a released call returns the held frame exactly as it would have read it.

**D8. Fixture rule: a one-shot successor open runs under the exclusive spawn gate from before the release (PR #125 residual 2).**

- Mechanism: fixtures retired or closed their `ServiceOwner` and at once called the one-shot `ServiceOwner::open` while holding only the shared spawn guard. `flock` ownership belongs to the open file description, and a child created by a sibling test between `posix_spawn`'s descriptor-table copy and its `exec` holds a duplicate of whatever this process had open, including the maintenance permit's or the old owner's owner-lock description. The successor's `try_lock` then sees `EWOULDBLOCK` and fails with "project already has a memory service owner; wait for its validated endpoint" although the old owner had fully released in the documented order (reap, endpoint retirement, lock release). The product is not affected in the same way: only a freshly elected owner process calls `ServiceOwner::open`, after its starter observed the owner lock free under the Start lock.
- Rule: `spawn_gate::excluding_spawns(held, restart)` drops the caller's shared guard, runs `restart` under the exclusive guard and downgrades that guard atomically to the caller's new shared guard; `ServedOwner::restart` retires, opens and serves the successor through it. The downgrade matters: a successor served inside the restart starts its 30 s idle timer at once, and a fresh shared acquisition would queue behind writers (other fixtures' restarts) that themselves wait for every running test. The first implementation did that, and at `--test-threads=8` the successor idled out before the fixture's recovery attached (verification round 7). The exclusive span starts before the release (retirement, close, a rejected open, or the teardown's retirement) and ends when the one-shot open has returned. `ServiceOwner::open` stays one-shot: no retry, wait or longer deadline, so a genuinely leaked owner lock still fails at once.
- Waiting for the exclusive guard happens before the retirement, while an old owner whose clients have closed may reach its own idle retirement. That is still ordered: `retire` then finds the task ended and reaps it, and any sibling spawn that began before that release holds its shared guard until its spawn call returns. By our understanding of std's Unix spawn (a `posix_spawn`, or a fork whose parent waits on a close-on-exec pipe; not checked against the std source here, which is not installed on this host), the child has exec'd by then, so the one-shot open under the exclusive guard cannot meet a duplicate.
- Cost: the gate is a fair `RwLock`, so a restart waits for every running spawner to drop its shared guard and queues new ones, inside the fixture's existing deadline (`fixture_deadline` includes at least two `QUERY_TIMEOUT`s). Code under the exclusive guard must not take a shared one (a nested shared acquisition would wait behind the writer forever); no restart site does, and `ServiceOwner::open` and the product take no gate. The shape matches the gated restart already in `lost_usage_reply_uses_natural_key_after_sibling_write_and_restart`.

## Catalogue dispositions

These entries are from `tmp/roadmap/phase2-handoff-2026-09-26-sources/flakes/flaky-tests.md`. That file numbers two entries "7" and two "8". They are labelled 7a/7b and 8a/8b here, so all 16 entries are accounted for.

| # | Run / job | Test (short) | Disposition |
|---|---|---|---|
| 1 | 36270828072 (Windows memory) | `provision::native_tests::persistent_held_descendant_exhausts_checked_recovery_and_preserves_stage` | Not fixed here: routed to the split Windows stage change (D6, branch `fix/memory-provision-stage-teardown`). The cause of its own diagnostic assertion is not established; re-examined there |
| 2 | 36276330774 (macOS) | runtime `dream::cancellation_tests::managed_lost_promotion_reply_…` (uncertain write) | H1, probable (no stage record) |
| 3 | 36278540871 (macOS) | tui `cli.rs` `dream` (uncertain write) | H1, probable (no stage record) |
| 4 | 36276764569, job 108500921447 (macOS) | two kuru-memory tests (uncertain write) | H1, probable (names only in drill logs) |
| 5 | 36285656438, job 108527295649 (macOS) | `facade::tests::preserved_candidate_conflict_reattaches_…` | H1, confirmed (`stage=cleanup … vendor=1105 reason=other`) |
| 6 | 36292320870, job 108544879759 (Windows) | `provision::native_tests::cancelling_checked_activation_recovery_drops_stage_before_cache_lock` | Not fixed here: routed to the split Windows stage change (D6, branch `fix/memory-provision-stage-teardown`) |
| 7a | 36294245109, job 108550021448 (Ubuntu p7) | kuru-delivery `bundle::recovery_tests::cancellation_after_dropping_error_headers_…` | Out of scope: kuru-delivery bundle lock, second change |
| 8a | 36296066873, job 108555025490 (macOS p3) | `store::usage_ledger::tests::permanent_branch_keeps_usage_out_of_main_…` | H1, confirmed (1105 in-use text at cleanup) |
| 7b | 36297681851 (macOS p1) | `service::tests::candidate_transition_query_preserves_open_conflict_…` | H1, probable (candidate path; no text recorded) |
| 8b | 36297681851 (macOS p3) | runtime `accounting_tests::candidate_compaction_stays_isolated_…` (accounting_tests.rs:2813) | H1, probable (candidate path; no text recorded) |
| 9 | 36304624151, jobs 108578990024 / 108582217193 (macOS p3) | kuru-delivery bundle lock `WouldBlock` | Out of scope: second change |
| 10 | 36310434585 attempt 1 (threads=4) | `inspection_owned_old_schema_blocks_writer_…` (jobs 108595181969, 108595181994), `cleanup_observation_error_keeps_actual_lifecycle_lease_…` (108595181983, 108595181996) | Deadline sub-items routed to the first-launch budget item. Threads stay 2 |
| 10 | same, job 108595182046 (macOS p1) | runtime `dolt_tests::canonical_dream_additions_survive_…` (cleanup 1105) | H1, confirmed |
| 11 | 36310434585 attempt 2 (threads=4) | `cleanup_observation_error_…` (108598442436, 108598442572), `inspection_owned_old_schema_…` (108598442477), runtime `ordinary_context_refuses_…` (108598442537), `managed_lost_promotion_reply_…` (108598442539): readiness or deadline | Routed to the first-launch budget item |
| 11 | same, job 108598442544 (macOS p2) | `facade::tests::managed_public_turn_lost_reply_…` (cleanup 1105); the canary `candidate_pool_retirement_observes_exact_server_sessions_before_rename`; runtime `hook_tests::cancelled_dream_abandons_candidate_…` | H1: confirmed; canary re-pointed at the product step `retire_branch_sessions` with assertions unchanged (D1 fallback); probable (abandon path) |
| 12 | PR #115 head 0c468e4e, job 108615291670 (macOS p2) | `facade::tests::managed_session_lifecycle_is_reversible_…` | H1, confirmed (v2.3.4; 92.63 s partition) |
| 13 | PR #115 head d7275608, job 108642168357 (macOS install) | `embedded_runtime.rs:1526` first launch timed out | Routed to the first-launch budget item |
| 14 | same run, job 108642168329 (Windows p1) | `managed_lost_promotion_reply_…` readiness deadline | Routed to the first-launch budget item |

## Risks / Trade-offs

- **The wait adds latency to every cleanup.** It is one processlist poll, which normally returns 0 immediately: first poll p50 0.4 ms idle, 0.8 ms under saturation. It is bounded by an existing deadline.
- **The processlist predicate sees only branch-qualified sessions.** Kuru's branch pools connect to `kuru/<branch>`, the same predicate the gated rename already relies on. A session on the unqualified `kuru` database would not be seen. Mitigation: the regression test injects a session through the same pool path the product uses.
- **Placing the wait in `Server::retire_pool` widens its effect.** Non-candidate callers also wait. This is acceptable, because they then gain the same guarantee under the same bound. D1's fallback keeps it store-level if an implementation finds a conflict.
- **The fixture invariant is stricter than a lock probe.** A store under a guarded root now needs a record even when its engine ran in another process and has long exited, so such fixtures must await quiescence explicitly. A missing await fails deterministically and names the fixture; it is fixed at the fixture, never by weakening the check.
- **The invariant will find more Class A/B sites than those listed.** The full suites showed 4-5 directory-removed-before-exit hits each. All must be fixed by ordering, never by excluding a test.
- **CI evidence of the natural trigger is statistical.** The observed CI rate was about 1 in 3-5 macOS runs, so a green set is weak evidence on its own. The deterministic regression test is the primary proof.
- **Exclusive restarts serialise the suite briefly.** Each of the ten restart sites (D8) waits for every running real-engine test to release its shared spawn guard. Measured cost is in verification round 7; a restart that exceeds its fixture deadline would name that deadline, not hide a leak.

## Open questions for the maintainer

These are recorded for a decision. This change takes neither; it changes only test support and tests.

- **Should `reconcile_outcome` consult storage before answering `InFlight`?** Today a `Running` receipt is answered `InFlight` from the in-process registry without a storage query (rpc.rs `reconcile_outcome`; the same registry check guards `candidate_outcome`, `candidate_transition_outcome` and `ledger_outcome`), even when storage already holds the committed receipt, which is written in the same transaction as the row. The alternative from the residual-1 diagnosis: query storage first and answer `Committed` when the receipt is found, `InFlight` only when it is not found and the request is still `Running`, else today's logic. Facts that bear on it: `indexed_logical_outcome` takes the store write lock, which the session-lifecycle worker holds from before dispatch until it ends, so the query would queue behind the worker under the existing `QUERY_TIMEOUT`/`OPERATION_TIMEOUT` with no new bound (verified only for the session-lifecycle path); and `registered_in_flight_write_cannot_be_reported_absent` would still see `InFlight`, because its request pauses before dispatch and before any row exists. Not implemented or tested.
- **Is a transient refusal right after a cancel acceptable to a user?** A product caller that runs `reconcile`/`recover_*` within milliseconds of a cancelled or failed managed write (for example kuru-runtime `engine.rs` after a lost annotation reply, and the `self.reconcile().await?` preambles) can meet the same window: its operation is refused with "remains uncertain", the fence is kept and the pending receipt retained, and the next attempt gets the definite answer. No data is misreported. Whether that transient refusal is acceptable to someone who acts right after a cancel is a product question; this change records it and does not treat it as a defect.
