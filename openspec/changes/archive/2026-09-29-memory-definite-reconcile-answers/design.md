# Design

## Context

Root cause. Each of the four outcome handlers in `packages/kuru-memory/src/service/rpc.rs` (`reconcile_outcome`, `ledger_outcome`, `candidate_transition_outcome`, also serving `SelectedAbandonOutcome`, and `candidate_outcome`) samples `ReceiptProgress::status` first and returns `InFlight` at once when the key is `Running`. Storage is never consulted, so a receipt that is already durable is reported as in flight. Separately, `RunningReceipt`'s `Drop` records `Completed` whether or not the handler returned, so a handler dropped by `abort_and_drain` or a panic reads as settled and a later guarded miss could answer Absent while the aborted effect (for example `abandon_checked`'s spawned task, or `abandon_candidate_ref`'s inline guarded SQL) may still start or be half-applied. Today that second state is not reachable for same-generation queries, because `abort_and_drain` only runs as the serve loop ends; the change makes `Completed` mean what the absence rule needs anyway.

Evidence labels used below: **[code]** read in this repository; **[src-dolt]** read in the pinned Dolt 2.3.5 source, not measured; **[inferred]** reasoning.

### Why a visible receipt proves a durable commit

- The receipt `INSERT` is the last statement of an explicit transaction, followed by `CALL DOLT_COMMIT('-Am', …)` and then the SQL `commit()` (`store.rs` `apply`). `apply`, `apply_session_lifecycle` and `usage_ledger::apply_change` contain no DDL; their only `CALL` is `DOLT_COMMIT` [code].
- Inside an explicit transaction nothing is visible to another session before `DOLT_COMMIT` [src-dolt].
- `DOLT_COMMIT` writes the commit object first and then publishes head and working set together through one root check-and-swap; on the journal store the journal is synced before the swap [src-dolt]. A reader sees the old root or the new root.
- A later SQL `COMMIT` is a no-op and a rollback only clears session state, so neither unpublishes [src-dolt]. Kuru rewrites no history [code].
- `DOLT_COMMIT` can fail after the root swap, and Kuru can drop the future at `QUERY_TIMEOUT` mid-statement; the row is then visible and durable while `apply` returns an error. The conclusion still holds because the commit object precedes publication, and the worker's `resolve_uncertain` finds the receipt and returns `Ok` [src-dolt + code].

The measuring test T8 checks both visibility halves on the real engine: not visible after the receipt `INSERT` and before `DOLT_COMMIT`; visible after `DOLT_COMMIT` and still present after the session drops without SQL `COMMIT`. It cannot measure the fsync ordering, which stays [src-dolt]. **If either half of T8 fails, the lock-free probe is removed and the change ships the settlement wait alone.**

### Pre-existing unverified assumption

`await_session_end` treats the absence of a session's `information_schema.processlist` row as proof that its running statement finished, and `resolve_uncertain`, hence the whole uncertain-write fence, relies on it. The assumption is that Dolt removes a session's process-list entry only after its running command returns. It is not verified by this change. This change adds no new reliance on it: the post-settlement guarded read runs `resolve_uncertain` exactly as today, and the lock-free probe does not use it. The spec delta states it.

## Goals / Non-Goals

**Goals:**
- Report committed whenever the exact durable receipt exists, including while the worker is still registered (main-view units and usage ledger).
- Otherwise wait for settlement, then answer from a guarded read, so in-flight is reported only when no definite answer exists within the deadline.
- Keep absence and open answers sound: they come only from a guarded read begun after a settled sample.
- A waiting query holds neither the write guard nor a store connection, and a departed client releases the wait promptly.

**Non-Goals:**
- No raised deadline, no added retry, no change to `startup_timeout_secs`.
- No weakening of isolation, ownership, recovery or the uncertain-write fence.
- No change to the client wire protocol or client-side reconciliation.
- Requests sent but not yet registered (status `Unknown`) are out of scope; a miss there stays still uncertain.
- No sleeps in tests.

## Decisions

**D1. Presence needs no settlement; absence does.** A lock-free read may only upgrade an answer to Committed, and ships only for main-view unit receipts (`probe_logical_receipt`, split from `indexed_logical_outcome`) and usage-ledger proofs (`inspect_proof_unguarded`). A probe hit is answered at once; `LogicalReceiptConflict` is returned as the fault; every other probe result (miss, error, own timeout) is discarded. The probe uses the store's existing pool, takes no lock and does not call `resolve_uncertain`. *Rejected:* a probe for candidate views, creation and transitions. Candidate lookups open pools on candidate refs, and `abandon_candidate_ref` refuses `Active` when that pool has another holder, so a probe would cause spurious refusals and weaken isolation; creation's `Open` answer constructs a handle that opens a branch session; and a lock-free `AbandonedReclaimed` is indistinguishable from an unrelated reclaim. These kinds wait and then read under the guard.

**D2. Handler order.** (1) `deadline = entry + HANDLER_BUDGET`; sample status. (2) Not `Running`: go to 6. (3) `Running` and probe allowed: probe. (4) `await_settlement` returns `Settled(sample)`, `Exhausted` (answer `InFlight`), or `ClientGone`/`ClientProtocolViolation` (marker error, no reply). (5) After `Settled`, where allowed, probe again so a Committed answer does not queue behind writers that took the guard during the wait. (6) If less than `REPLY_MARGIN` remains, skip the guarded read and answer StillUncertain (or `InFlight` if still `Running`, which keeps the rule total). (7) Otherwise the existing guarded read under `timeout_at(deadline)` with the existing match arms. One shared helper covers steps 1-6 with an optional probe closure. Invariant, as a comment on the helper: a negative or open answer comes only from a guarded read that started after a `Completed` sample or under a changed generation; a `Running` sample never carries into a negative answer.

**D3. Deadlines are derived and only lowered.** `REPLY_MARGIN = OPERATION_TIMEOUT − QUERY_TIMEOUT` (5 s), `HANDLER_BUDGET = OPERATION_TIMEOUT − REPLY_MARGIN` (30 s), `PROBE_BUDGET = ORDINARY_POOL_WINDOW` (2 s, already the pool's acquire timeout), with a const assertion `HANDLER_BUDGET > REPLY_MARGIN + PROBE_BUDGET`. The wait ends at `min(now + settlement_wait_limit, deadline − REPLY_MARGIN − PROBE_BUDGET)`, at most about entry + 23 s. The guarded read, previously bounded by `OPERATION_TIMEOUT`, is now bounded by the handler deadline. The client's reply deadline is unchanged. `settlement_wait_limit` defaults to `QUERY_TIMEOUT` (never the binding term) and has a `#[cfg(test)]` setter; zero is a budget, not a sleep. *Rejected:* a new constant or a raised deadline. A worker's own SQL deadline starts only after it acquires the guard and connection, so settlement can outlast any budget here; the consequence is more in-flight answers, never wrong ones.

**D4. The settlement wait (lead condition 1).** `ReceiptProgress` gains one shared `tokio::sync::Notify`. `await_settlement` loops: enable the `Notified` future, then re-sample (no lost wakeups); `select!` on the notification, `sleep_until(wait_end)` (one final re-sample, then `Exhausted`), and a cancel-safe one-byte `read` on the client stream (`Ok(0)`/`Err` → `ClientGone`; `Ok(1)` → `ClientProtocolViolation`, since framing is lost). Every `RunningReceipt` drop calls `notify_waiters()`. `respond` widens to `S: AsyncRead + AsyncWrite + Unpin`, which both callers satisfy, and passes the stream into the outcome handlers. On `ClientGone` or violation `respond` returns an `io::Error` (`BrokenPipe` / `InvalidData`) before the fault-classification arm, so no warning, no `StorageFailed` mapping and no reply; the serve loop exits and drops the frame permit and attachment state, and `attach()` treats it as peer-closed.

While waiting the query holds: no write guard; no store connection (the bounded probe returns its connection before the wait); 1 MiB of frame budget, one attachment slot and one `Retirement.active` count, as any open attachment does, all released when the attachment ends; and only its own connection's serial loop. Outcome queries are reads and hold no `RunningReceipt`, so a departed client changes no settlement state.

**D5. Settlement and the sticky unsettled mark.** The `RunningReceipt` is hoisted out of the `processed` block: compute the key, `begin`, run the call, then `settle()` on both `Ok` and `Err` values; an abort anywhere drops it unsettled. `ProgressState` gains `unsettled`, `unsettled_order` and `unsettled_overflow`. An unsettled drop removes the key from `completed` and marks it; a settled drop inserts into `completed` only when not marked and not overflowed. `status` resolves `Running`, then `Unknown` if marked or overflowed, then `Completed`, then `Unknown`. Marks are bounded by `COMPLETED_RECEIPT_WINDOW`; on overflow a generation-wide flag makes every non-`Running` key `Unknown`, the safe direction. *Rejected:* evicting single marks, which would let a later same-key request re-insert `Completed` while the aborted effect might be pending; per-key notifies, which the re-sample loop makes unnecessary.

**D6. Clearing the fence on Committed while the writer still holds the guard is safe.** Nothing is ambiguous after Committed. Every mutation takes the write guard and runs `resolve_uncertain` first, so the next mutation queues behind the still-running worker; the cancelled attachment is already unusable. Candidate unit writes keep the fence after Committed until the candidate is reattached through the guarded `CandidateOutcome` call, unchanged.

**D7. Test accommodations.** Remove the 13 `replied` waits, `ReplyBarrier::wait_replied` and (as the last step) `ReplyPause::replied`. Convert four 20 ms outcome polls to single calls; their preceding visibility waits stay, because they are the registration proof for out-of-process owners. Keep the two raw-frame polls in `service.rs`: they drop the stream right after writing the frame with no proof of registration, and their first still-uncertain iterations are exactly the unregistered window this change leaves out of scope. No cancel moves to `sent`.

**D8. Test seams (`#[cfg(test)]`).** `SettlementPause` (after the value, before `settle`), `WaitEvent::{Entered, Ended(reason)}` on an unbounded channel on `ReceiptProgress`, `set_settlement_wait`, `waiters_for_test`, `Retirement::active_for_test`, and per-store (not process-global) apply hooks `after_receipt_insert` and `after_dolt_commit` for T8. Wall-clock bounds only fail a test, never pass it.

## Risks / Trade-offs

- [The probe's soundness rests on Dolt source until T8 runs] → T8 measures both halves; if either fails, remove the probe and ship the wait alone. Every other decision stands without it.
- [The process-list assumption stays unverified] → pre-existing and shared with the fence; stated in the spec; this change adds no reliance on it.
- [Reconcile latency: up to about 23 s before in-flight, instead of immediately] → only while a fenced write is actually in progress; UI-path callers (`kuru-runtime` engine, `kuru-tui` CLI) see it only then.
- [Guarded-only kinds can queue behind writers that took the guard during the wait] → if the margin rule then skips the read the answer is still uncertain; safe, and the fence holds. The same queueing already applies to queries arriving after `Completed`.
- [A waiting query defers idle retirement and makes a concurrent selected abandon refuse `Active`] → as any open attachment does; bounded by the wait end.
- [Late replies after long request reads] → pre-existing: when the owner's request-read lag exceeds `REPLY_MARGIN` the client sees a transport error and the fence holds.
- [Behaviour change for dropped handlers] → a dropped handler followed by a guarded miss now answers still uncertain instead of absent; intended.
- [Windows named-pipe close semantics for the client-gone arm] → both `Ok(0)` and `BrokenPipe` map to `ClientGone`; a native Windows run of T13 is required evidence.
