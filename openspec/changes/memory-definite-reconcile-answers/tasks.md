# Tasks

## 1. Regression tests first (red before the product edit)

- [ ] 1.1 Add the test seams (`SettlementPause`, `WaitEvent`, `set_settlement_wait`, `waiters_for_test`, `Retirement::active_for_test`, per-store apply hooks) and the regression tests T1, T2, T4, T5, T6, T7, T10, T11, T12, T13 and T14, and verify each fails on the unfixed handlers with the expected wrong answer (`InFlight`, `Absent`, or no `ClientGone` event), recording the output (verification 2-5).
- [ ] 1.2 Add T8 (both halves) and adapt T3 to a zero settlement budget, and verify both pass as guards; if either half of T8 fails, record it and drop the lock-free probe from tasks 2.1-2.2 (verification 1.1, 1.2, 4.4).

## 2. Package A: owner-side product change (kuru-memory)

- [ ] 2.1 Split the main-view receipt match out of `indexed_logical_outcome` as unguarded `probe_logical_receipt`, and verify T8 exercises it.
- [ ] 2.2 Split `usage_ledger::inspect_proof` into a guarded wrapper and `inspect_proof_unguarded`, and verify the guarded callers are unchanged in behaviour (existing ledger tests pass).
- [ ] 2.3 Add settlement state to `ReceiptProgress` (shared `Notify`, `settlement_wait_limit`, sticky unsettled marks with overflow), `RunningReceipt::settle`, the rewritten `Drop` and `status`, and verify T14 passes.
- [ ] 2.4 Add `REPLY_MARGIN`, `HANDLER_BUDGET` and `PROBE_BUDGET` derived from existing constants with the const assertion, and verify no deadline is raised by reading the diff.
- [ ] 2.5 Hoist `RunningReceipt` out of the `processed` block with `SettlementPause` and `settle`, widen `respond` to `AsyncRead + AsyncWrite`, add the shared outcome helper and `await_settlement`, rewrite the four outcome handlers without their early `InFlight` returns, and add the client-gone and protocol-violation short-circuit; verify T1-T13 pass and 4.6's existing still-uncertain cases still pass.
- [ ] 2.6 Run `mise run //packages/kuru-memory:test`, `mise run lint`, `mise run format:check` and `mise run typecheck`, and record the results in verification.

## 3. Package B: test conversions and the fence test

- [ ] 3.1 Remove the 13 `replied` waits and `ReplyBarrier::wait_replied`, fixing adjacent comments, and verify the converted tests pass (verification 6.1).
- [ ] 3.2 Convert the four 20 ms outcome polls to single calls, keeping their visibility waits, and keep the two raw-frame polls in `service.rs`; verify they pass (verification 6.2, 6.3).
- [ ] 3.3 Add T9 to the `facade.rs` tests and verify the fence trips and the pending receipt is retained (verification 4.5).
- [ ] 3.4 Run `mise run //packages/kuru-memory:test` and `mise run //packages/kuru-runtime:test`, then lint and format, and record the results.
- [ ] 3.5 Final package-A commit after B: remove `ReplyPause::replied`, its notify and doc text, and verify no use remains and the workspace builds and lints (verification 6.4).

## 4. Package C: documentation, gates and archive

- [ ] 4.1 Update `docs/development.md` (lost-reply test guidance; memory service protocol: an outcome query may wait for settlement within the operation deadline minus a reply margin and ends the wait when its client leaves) and `docs/memory.md` (Kuru answers from the durable receipt as soon as it exists, otherwise waits within the existing deadline), and verify `mise run docs:check` passes.
- [ ] 4.2 Run `mise run coverage` and record the workspace line percentage against the 90% gate (verification 7.3).
- [ ] 4.3 After push, record CI native partitions including Windows T13 and Linux arm64, and the repeated-run flake measurement, or name each as unrun with its reason (verification 5.4, 7.4, 7.5).
- [ ] 4.4 Fill verification with observed results only, complete tasks, validate strictly, run `mise run cospec -- archive memory-definite-reconcile-answers`, and verify the archive directory exists before the final branch commit.
