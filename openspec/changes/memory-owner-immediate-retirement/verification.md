# Verification

Nothing below has been run yet (authored 2026-09-29 before implementation). Every row is open. Tests sleep nowhere: each assertion follows an awaited event and each wait is bounded only by the fixture's existing backstop. Windows rows are unverified on the macOS authoring host until native CI runs them.

## 0. Measurements (early; stop rule)

- [ ] 0.1 @benchmark (agent) V1 close duration on macOS and Ubuntu, then Windows in CI: CLI command exit to Owner lock release, several runs per platform -> typical and worst observed figures recorded per platform, with host and commit
- [ ] 0.2 @benchmark (agent) V2 reopen time on macOS and Ubuntu, then Windows in CI: command start to memory ready, both after a completed close and when landing during a close -> figures recorded; if a reopen right after a close typically waits more than about one second in total, implementation stops before merge and reports
- [ ] 0.3 @runtime (agent) V3 the Windows error code a client sees when its queued pipe instance is closed during shutdown, from T6's native Windows leg -> code recorded (233 expected by reading, unverified)
- [ ] 0.4 @benchmark (agent) V4 replacement time after a cancel (T8) on each native platform -> figures recorded and well inside the asserted 15 s < 35 s margin

## 1. The owner retires as soon as it is unused [critical]

- [ ] 1.1 @regression (agent) T1 served owner `Starter(t)` with a 1 h first-attachment deadline; attach presenting `t`; detach; await `EnteredEmpty { reached: true }`; reap bounded only by the backstop -> endpoint absent, lifecycle lease and Owner lock free; fails on main, where the same body served with a 1 h idle interval never exits before the backstop
- [ ] 1.2 @regression (agent) T2 not-reached owner: a wrong-secret connection, then a valid non-starter attach and detach, each followed by `AttachmentJoined { 0 }` and `EnteredEmpty { reached: false }`; the starter then attaches with `t` to the published generation; its detach gives `EnteredEmpty { reached: true }` and the owner finishes -> a non-starter cannot retire a fresh owner; fails on main, where a detach starts retirement regardless of who attached
- [ ] 1.3 @integration (agent) T3 `Starter(t)` with the deadline already passed and no client -> `serve` returns `Ok`, quiescence recorded, warn-level log present
- [ ] 1.4 @regression (agent) spawned owners (decision 4 condition): rewritten `independent_clients_elect_one_real_process_and_retire_after_both_detach` and `separate_cold_starters_share_one_owner_and_preserve_both_writes`, with really spawned owners -> one generation while both clients are attached; after both detach, `await_owner_release` returns without any idle wait, and a successor holds both rows; fails on main (no release inside the 30 s grace)
- [ ] 1.5 @e2e (agent) T16 rewritten `sequential_commands_release_clients_..._start_a_fresh_owner_each_time` (`apps/kuru-tui/tests/cli.rs`) -> each CLI command's owner releases its lock after the command exits (awaited, not slept), the next command starts a fresh owner, and reopen and close figures are recorded for 0.1 and 0.2
- [ ] 1.6 @integration (agent) T18 rewritten `idle_accept_deadlines_do_not_close_live_attachment`: short recheck interval; await three `LockRechecked` -> the live attachment still reads; after it drops, the owner exits

## 2. Pending work holds the owner [critical]

- [ ] 2.1 @integration (agent) T4 attachment A's write paused by `RegisteredPause`; A drops its stream; B detaches -> ordered log shows `AttachmentJoined { 1 }`, and after release `AttachmentJoined { 0 }` then `EnteredEmpty { reached: true }`, with no earlier `EnteredEmpty`; the write is durable in a successor
- [ ] 2.2 @integration (agent) T5 attachment L holds the dream lease while others close -> same ordered-log check; after L closes the owner retires and the candidate ref is inspectable by a successor

## 3. Shutdown order and racing clients [critical]

- [ ] 3.1 @integration (agent) T7 `ClosePause` at `AfterEndpointRetire` and `AfterReap` -> at the first, endpoint record absent while Owner lock and lifecycle lease are busy; at the second, lifecycle lease free and Owner lock still busy
- [ ] 3.2 @integration (agent) T6 native Unix and Windows: pause at `BeforeListenerDrop`; a raw queued stream's `connect_handshake` after release fails `is_peer_closed`; then `attach_or_start` (executable `test_supervisor()`) observes Start busy and attaches to a new generation only after `AfterReap` is released; repeated with the pause at `AfterEndpointRetire` and `AfterReap` -> no error from the race, no overlapping owners; Windows leg records the error code (0.3)
- [ ] 3.3 @regression (agent) T6m pause at `BeforeListenerDrop`; `acquire_maintenance_permit` meets the dropped listener -> `None`, then the permit is acquired after unlock; fails on main, where the peer-closed handshake is an error
- [ ] 3.4 @unit (agent) T12 hold Start with no owner; `attach_existing` with a 1 s startup timeout -> returns the "did not publish" error without probing the Owner lock; after Start is released it returns `Ok(None)`
- [ ] 3.5 @unit (agent) T13 explicit unlock -> Unix: a second acquire succeeds after `ServiceLock::release` while a `try_clone` of the descriptor is still open; Windows: reacquire after release (Windows unverified until CI)
- [ ] 3.6 @unit (agent) T14 Windows pipe marker -> all instances busy gives kind `TimedOut`, `is_no_free_instance` true and the unchanged message; a missing name gives `NotFound` with the marker false; `try_attach` maps the marker to `None` (Windows-only; compiled by the Windows-target lint on macOS, behaviour unverified until CI)
- [ ] 3.7 @unit (agent) T15 argument and hello shapes -> `service_arguments` with a token gives ten arguments; parse accepts nine and ten and rejects an eleventh and a non-UUID; a hello without a token serializes byte-identically to today; a hello with a token is accepted by the owner and marks it reached
- [ ] 3.8 @integration (agent) existing `owner_reaps_real_dolt_before_retiring_endpoint_and_lock`, renamed `owner_retires_endpoint_then_reaps_before_releasing_its_lock` -> passes with the new order and the explicit release probe

## 4. Cancelled calls keep a running client's owner (Option B) [critical]

- [ ] 4.1 @regression (agent) T8 served owner (`AnyAttachment`), one writable `MemoryStore` session as sole client; pause the next reply on the primary; await the paused reply; drop the call -> replacement `installed`, then `AttachmentJoined { 1 }`; the next call succeeds on the same generation; no `EnteredEmpty` in the log; fails on main (owner retires, next call fails)
- [ ] 4.2 @integration (agent) T8b cancel during dispatch under `RegisteredPause`; await `installed`; release -> the owner's reply write fails quietly; `AttachmentJoined { 1 }`; `reconcile` on the same generation answers committed; the request was not resent and the fence applied
- [ ] 4.3 @integration (agent) T8c replacement paused after connect; `MemoryStore::close()`; release -> replacement discarded; `AttachmentJoined` down to 0 and `EnteredEmpty { reached: true }`
- [ ] 4.4 @integration (agent) T8d owner filled to `MAX_ATTACHMENTS`; cancel a read paused by `DispatchPause` -> replacement hook reports `failed`; after one bare attachment closes and the read is released, the next call's lazy connect is logged `AttachmentAccepted` before the old primary's `AttachmentJoined`, and the call succeeds on the same generation
- [ ] 4.5 @integration (agent) T9 retained open candidate ref; cancel a paused non-mutating read; await `installed` -> the same session's `abandon_candidate_ref` is refused `Active`; after `AttachmentJoined { 1 }` the same abandon succeeds
- [ ] 4.6 @integration (agent) T10 read-only session plus one writable session; cancel a paused read on the read-only session; release -> the first event after release is `AttachmentJoined { 1 }`, not `AttachmentAccepted` (no hold, no replacement)
- [ ] 4.7 @unit (agent) held-stream unit test and the margin -> `ServiceAttachment::close()` clears both the live and held streams; the build contains the `const` assertion `3 * HANDSHAKE_TIMEOUT < OPERATION_TIMEOUT` (a build with the margin crossed fails to compile, checked once by hand and recorded)
- [ ] 4.8 @integration (agent) existing `cancelled_client_call_invalidates_its_connection` -> still passes: the cancelled stream is never reused for another exchange

## 5. Checked recovery ends on the verified successor

- [ ] 5.1 @integration (agent) T11 pending unit write; G1 retired under the fixture; G2 served `AnyAttachment`; reconcile reaches G2 -> G2's log has no `EnteredEmpty`; reopen returns a view on G2's generation
- [ ] 5.2 @integration (agent) T11a sole-attachment effect of the successor slot -> while the slot is filled a candidate abandon from that session is refused `Active` (active == 2); after the reopen consumes it the same abandon passes the reservation; a session close drops the slot and the owner retires

## 6. Tests and fixtures no longer rely on the grace

- [ ] 6.1 @e2e (agent) T17 logged-owner CLI test -> the diagnostic stderr of whichever owner the CLI child elects is captured through the forwarded test-support environment hook, and the `candidate_owner stage=ref_inspection ... fault=ref_rejected` line is asserted
- [ ] 6.2 @integration (agent) Windows `starter_job_exit_preserves_independent_owner_and_surviving_client` and `denying_outer_job_contains_owner_until_close_then_recovery_succeeds` rewritten per design -> pass natively on Windows (unverified until CI); no endpoint-absence polling remains
- [ ] 6.3 @manual (agent) audit by content every test that reads endpoint absence as "reaped", and every `SERVICE_IDLE_TIMEOUT` reference -> none remain; each replaced by `await_owner_release` or an observer event; the list is recorded
- [ ] 6.4 @manual (agent) search the diff for added sleeps in tests -> none

## 7. Repository checks

- [ ] 7.1 @integration (agent) `mise run //packages/kuru-platform:lint`, `:test`; `mise run //packages/kuru-memory:lint`, `:test`; `mise run //apps/kuru-tui:test`; `mise run //packages/kuru-runtime:test` -> pass locally on macOS
- [ ] 7.2 @integration (agent) `mise run lint` (host and Windows target), `format:check`, `typecheck`, `docs:check`, `mise run cospec -- validate memory-owner-immediate-retirement --strict` -> pass
- [ ] 7.3 @integration (agent) CI native legs (macOS, Ubuntu, Windows) including coverage at the 90% gate and previous-release update acceptance -> green; no new protocol refusal
