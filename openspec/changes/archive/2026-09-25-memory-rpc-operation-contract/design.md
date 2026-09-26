## Context

`packages/kuru-memory/src/service/rpc.rs` carries the typed memory service
protocol (1.7). Three functions decide how an operation is handled:

- `ServiceCall::may_mutate` is exhaustive over `ServiceCall`, but its `View`
  arm is a non-exhaustive `matches!` over `ViewOperation` and its `Ledger` arm
  is `!matches!(.., Session)`. The facade uses it to take the shared mutation
  lock and the lost-reply fence.
- `ServiceCall::unit_receipt_method` ends in `_ => None` twice. It names the
  durable logical receipt stored with a unit write.
- `receipt_progress_key` special-cases candidate creation, usage-proof ledger
  calls, candidate transitions and selected abandonment, then falls back to
  `unit_receipt_method`.

`dispatch` repeats the unit-receipt view lookup, and `RetireIfIdle`,
`TryAcquireDreamLease` and `AbandonCandidateRef` each spell out their own
attachment resource predicate. No test pins the wire surface to
`PROTOCOL_MINOR`.

## Goals / Non-Goals

**Goals:**

- Adding a `ServiceCall`, `ViewOperation` or `LedgerOperation` variant fails to
  compile until its mutation, receipt and reply-budget class are decided.
- Receipt-progress registration is an exhaustive match on the receipt class.
- A wire-surface change fails a test until `PROTOCOL_MINOR` is bumped and the
  fixture is deliberately regenerated.
- Pin today's classification of every variant in a table.

**Non-Goals:**

- No wire, protocol-version, receipt, lock, deadline or refusal change.
- No `ServiceFault` classification consolidation (audit R5); it is separate.
- No Local-through-dispatch routing (R2), file splits (R3) or new fixtures (R4).
- The client-side pending-proof extraction in `facade.rs` keeps its payload
  matches; it already follows the same receipt classes.

## Decisions

1. **A static `OperationContract` value per variant.** `ServiceCall::contract()`
   returns `{ mutation: Mutation, receipt: Receipt, reply: ReplyBudget }`, all
   `Copy`. `View` delegates to `ViewOperation::contract()` and `Ledger` to
   `LedgerOperation::contract()`. Every match lists each variant with no
   wildcard. Rejected: per-instance contracts that borrow payload fields; they
   make the golden table awkward and the static class is what must be decided.
2. **Receipt classes mirror the existing proofs.** `Receipt` is `None`,
   `Unit(&'static str)`, `CandidateCreation`, `CandidateTransition`,
   `SelectedAbandon` or `UsageProof`. `receipt_progress_key` matches on it and
   extracts payloads with `let … else`, keeping each existing `context` and
   `ensure!` text. The usage-proof arm keeps the `operation.proof()?` error path
   and today's `Ok(None)` fallthrough if a proof is absent. Rejected: keeping
   `unit_receipt_method` as the fallback; it is exactly the silent default.
3. **Mutation and receipt remain separate fields.** `RetireIfIdle` mutates with
   no receipt; it is the only allowed combination, pinned by an invariant test.
   `TryAcquireDreamLease` changes only attachment state and stays read-only.
   Rejected: deriving mutation from receipt, which would hide that exception.
4. **One reply-budget class today.** `ReplyBudget::Operation` maps to the
   existing `OPERATION_TIMEOUT` and is used for the attached client's reply
   read. Frame-level read/write limits keep their own constant. This gives a
   longer-running call (backup) a typed home without changing any deadline.
5. **Two attachment predicates.** `holds_handles()` is candidates or exports;
   `holds_resources()` adds the dream lease. `RetireIfIdle` and
   `TryAcquireDreamLease` use `holds_resources()` (identical to today; the
   latter only evaluates it while no lease is held). `AbandonCandidateRef`
   keeps `holds_handles()`: today a lease-holding attachment is not refused, and
   changing that is a wire-observable refusal outside a refactor. Rejected:
   one lease-inclusive predicate at all three sites. Session claims must decide
   whether selected abandonment should also exclude lease or claim holders.
6. **Protocol pin from serde itself.** The fixture records the protocol version,
   the JSON shape (sorted keys, leaf values replaced by type names) of one
   sample per request variant with every optional field populated, and the tag
   lists of `ServiceValue`, `ServiceResponse`, `ServiceFault`, `OutcomeStatus`,
   `CandidateCreationOutcome`, `CandidateTransitionKind` and
   `CandidateTransitionResult`. Tag lists come from serde's unknown-variant
   error for each derive, and the request samples must cover exactly those
   tags. Rejected: a schema-generation dependency (new pinned dependency for a
   test) and a hand-kept ordinal count (a forgotten count is silent).
7. **Enforcement lives in the regeneration path.** A mismatch fails with the
   file to regenerate, `KURU_BLESS_PROTOCOL_PIN=1`, and whether the version must
   first be bumped. The bless path writes the fixture only when the recorded
   version differs from the current one, so an unchanged version never absorbs
   a changed surface; hand editing remains a visible, reviewable act.

## Risks / Trade-offs

- [Serde's unknown-variant message format changes on a dependency update] →
  The parser fails loudly and names the enum; pins are exact, so this appears
  only in a deliberate update.
- [Samples cover one shape per variant; nested enums in `kuru-core` types show
  only the sampled variant] → The pin covers every request variant's field
  names and sampled nested shapes; the failure message says that shared types
  riding the wire also trigger it intentionally.
- [Golden fixture churn on every protocol change] → Intended; the fixture is the
  reviewable record of the wire change.
- [A contract mistake on day one] → The classification table was derived from
  the pre-refactor functions and passes against both the old and new code.

## Seam ownership

`service/rpc.rs` owns the operation contract, attachment predicates and pin
test. The facade continues to consume `may_mutate` and
`unit_receipt_fingerprint`; `service.rs` owns `PROTOCOL_MAJOR`/`PROTOCOL_MINOR`
and the handshake, read but not modified by the pin. No runtime state moves.

## Integration contract

The external contract is the memory service wire protocol between a Kuru client
and a possibly older or newer project owner. It is unchanged at 1.7. The new
fixture records that surface so that future changes to it are coupled to a
minor bump and the existing handshake refusal.
