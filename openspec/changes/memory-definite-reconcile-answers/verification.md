# Verification

Nothing below has been run. Every row is acceptance evidence authored before implementation; each `[ ]` becomes `[x]` only with observed output, or `[~] … -> defer: <reason>`. "Red at HEAD" is the expected pre-fix result by reading (design D2, D5), and must be observed, not assumed.

## 1. Publication proof for the lock-free probe [critical]

- [ ] 1.1 @integration (agent) T8 negative half, `store.rs` tests: a main-view logical write with the `after_receipt_insert` apply hook; at that point the hook reads through the unguarded probe on a pooled connection -> the probe returns `false` (the receipt is not visible after its `INSERT` and before `DOLT_COMMIT`). Guard: expected green once the apply hooks land (they are part of this change, so it cannot run on the unfixed tree). If it fails, the probe is removed and the wait ships alone (design, Context).
- [ ] 1.2 @integration (agent) T8 positive half: at `after_dolt_commit` the hook reads the same way, then injects an error so the connection drops without SQL `COMMIT` -> the probe returns `true`; the write resolves through `resolve_uncertain` to `Ok`; after `await_session_end` the row is still present and the appended data is readable. If it fails, the probe is removed.

## 2. Committed is reported whenever durable evidence exists [critical]

- [ ] 2.1 @regression (agent) T1 unit visible while Running, `service.rs`: append paused at `SettlementPause` -> outcome is `Committed` while the writer's serve task is unfinished and no `WaitEvent::Entered` was sent; after release, `Committed` again. Red at HEAD (answers `InFlight`).
- [ ] 2.2 @regression (agent) T2 unit waits then Committed: append paused at `RegisteredPause`; query spawned; await `Entered` -> query unfinished; release; query returns `Committed`; `Ended(Settled)` observed. Red at HEAD.
- [ ] 2.3 @regression (agent) T5 usage ledger: T1 and T2 shapes with `Ledger` admit and `LedgerOutcome` -> `Committed` both ways. Red at HEAD.
- [ ] 2.4 @regression (agent) T11 probe keeps the conflict fault: append paused at `SettlementPause`; query its ID with a different argument digest -> `ReceiptConflict` fault, not `Committed` and not `InFlight`. Red at HEAD.

## 3. Guarded-only kinds wait, then answer definitely

- [ ] 3.1 @regression (agent) T6a promote: paused at `SettlementPause`, query connected after `entered`, await `Entered`, assert unfinished -> after release `Promoted { revision == target }`; zero-budget variant answers `InFlight`, never an open answer. Red at HEAD.
- [ ] 3.2 @regression (agent) T6b `AbandonCandidate`, same shape -> after release `Abandoned`; zero-budget variant `InFlight`, never `Abandoned`. Red at HEAD.
- [ ] 3.3 @regression (agent) T6c `AbandonCandidateRef` through a managed owner (`serve_attached` with a `Retirement`), same shape -> after release `Abandoned`; zero-budget variant `InFlight`, never `Abandoned`. Red at HEAD.
- [ ] 3.4 @regression (agent) T7 candidate creation: `BeginCandidate` paused at `SettlementPause`; query awaits `Entered` -> unfinished until release, then `Open` with the expected base and branch. Red at HEAD.

## 4. Absence and the uncertain-write fence stay sound [critical]

- [ ] 4.1 @regression (agent) T4 Absent only after settlement: a receipt-bearing `AppendMessage` with a blank namespace (rejected by validation before any SQL write), paused at `RegisteredPause`; query awaits `Entered` -> after release the query returns `Absent`; the same request at zero budget returns `InFlight`. Waiting half red at HEAD.
- [ ] 4.2 @regression (agent) T10 dropped handler is not Completed: write paused at `RegisteredPause`; abort its serve task -> same-generation outcome for that ID is `StillUncertain`, not `Absent`. Red at HEAD.
- [ ] 4.3 @unit (agent) T14 settlement bookkeeping on `ReceiptProgress` directly, no Dolt -> (a) A settles, B same key drops unsettled: `Unknown`, and a later settled C stays `Unknown`; (b) A and B concurrent, B drops unsettled, A settles last: `Unknown`; (c) overflow of the unsettled window makes an unrelated `Completed` key read `Unknown`; (d) every drop wakes a waiter. (a)-(c) red at HEAD.
- [ ] 4.4 @integration (agent) T3 guard at zero budget: the existing `registered_in_flight_write_cannot_be_reported_absent` with `set_settlement_wait(ZERO)` before the query, body and assertions otherwise unchanged -> `InFlight`, then `Committed`. Guard: green before and after.
- [ ] 4.5 @integration (agent) T9 fence trips on a genuinely uncertain outcome, `facade.rs` tests: a candidate unit write loses its reply; a sibling completes a selected abandon (retrying while the owner refuses `Active`) that reclaims the exact ref -> recovery fails with "candidate unit outcome remains uncertain"; a following mutation on any clone fails at `ensure_mutation_allowed`; the pending receipt is retained. Guard: green before and after.
- [ ] 4.6 @integration (agent) existing still-uncertain coverage (at a96a16ae: `service.rs` 3418, 3984, 4118-4123, 4530 and `facade.rs` 5359-5375) -> unchanged and passing.

## 5. A waiting query blocks no one and releases on client exit (lead condition 1) [critical]

- [ ] 5.1 @regression (agent) T12 other sessions proceed: write W paused at `RegisteredPause` (before the guard); query Q awaits `Entered`; on two further attachments a read (`Get` or `HistoryWindow`) and an `AppendMessage` with a new ID -> both complete while Q is unfinished and W is still paused; release W; Q returns `Committed`. Red at HEAD (no wait exists).
- [ ] 5.2 @regression (agent) T13 disconnect: W paused at `RegisteredPause`; Q served by `rpc::serve_attached` in a spawned task with a test-owned frame-budget `Semaphore` and `Retirement`, holding a `RetainedAttachment`; await `Entered`; drop the client stream -> `WaitEvent::Ended(ClientGone)` received while W is still paused; the serve task ends with `Ok` or a peer-closed error; `available_permits() == FRAME_BUDGET_MIB`; `active_for_test()` back to baseline; `waiters_for_test() == 0`; release W, W commits, a fresh query answers `Committed`. No sleep; promptness observed through the event. Red at HEAD.
- [ ] 5.3 @regression (agent) T13 cancel: same as 5.2 but abort the client task mid-call -> same assertions. Red at HEAD.
- [ ] 5.4 @runtime (agent) T13 on native Windows (named-pipe close surfaces as `Ok(0)` or `BrokenPipe`) -> both variants pass in the Windows native partitions; run and job ids recorded.

## 6. Test accommodations removed

- [ ] 6.1 @integration (agent) remove the 13 `replied` waits (`facade.rs` ×11, `store/migrations.rs` ×1, `kuru-runtime/src/hook_tests.rs` ×1) and `ReplyBarrier::wait_replied` -> the converted lost-reply tests pass, each with a single definite reconcile after its visibility wait.
- [ ] 6.2 @integration (agent) convert the four 20 ms outcome polls (`facade.rs` ×3, `kuru-runtime/src/dream.rs` ×1) to single calls, keeping their preceding visibility waits -> pass.
- [ ] 6.3 @integration (agent) the two raw-frame polls in `service.rs` (promotion and abandonment) are kept unchanged -> `git diff` shows them unchanged, and they pass.
- [ ] 6.4 @integration (agent) `ReplyPause::replied`, its notify and doc text removed in the final package-A commit -> `rg -n 'replied' packages/` shows no remaining use; workspace builds and lints.

## 7. Repository gates

- [ ] 7.1 @integration (agent) `mise run //packages/kuru-memory:test` and `mise run //packages/kuru-runtime:test` -> pass, no test excluded; counts recorded.
- [ ] 7.2 @integration (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate memory-definite-reconcile-answers --strict` -> all exit 0.
- [ ] 7.3 @integration (agent) `mise run coverage` -> workspace lines at or above 90% with no exclusions; percentage recorded.
- [ ] 7.4 @runtime (agent) CI at the pushed head: Ubuntu, macOS, Windows (x64 and arm64) and Linux arm64 native memory partitions -> green; run ids recorded.
- [ ] 7.5 @benchmark (agent) repeated-run flake measurement of the new service tests (T1-T13) -> no failures across the recorded iteration count; command and counts recorded.
