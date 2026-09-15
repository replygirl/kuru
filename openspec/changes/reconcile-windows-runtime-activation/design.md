## Context

Release run `34925918389` reached the staged Windows acceptance after the real
Dolt version probe, then `MoveFileExW` returned access denied. The existing
reconciliation reported `source=same-identity` and `destination=absent`, which
proves that particular move did not occur, but `files.rs` currently treats only
the opposite checked state as conclusive. Reviews found no Kuru-owned probe
handle leak, and the actual owner of the denial is unknown.

## Goals / Non-Goals

**Goals:**
- Represent moved, proven no-move, and unresolved publication outcomes without
  losing the original typed publication and OS error.
- Permit a narrow Windows bundled-runtime activation retry only for access
  denied plus proven no-move, under the existing stage and cache lock.
- Bound recovery to two seconds with 20-millisecond spacing and release source,
  stage and lock authority together if the async caller is cancelled.

**Non-Goals:**
- Explain or suppress the external handle owner.
- Retry other memory directory moves, other OS errors, occupied or rebound
  destinations, or observation failures.
- Change the embedded archive, version probe, global startup deadline, release
  workflow, platform move primitive, or non-Windows behavior.

## Decisions

The filesystem boundary will expose a private typed no-move proof only when an
initially uncertain publication error is followed by a source that retains its
held identity and name and an absent destination. That proof will reclassify the
underlying `PublicationError` as rejected while preserving its original OS error.
A destination with the held source identity and an absent source remains a
reconciled successful move. Every other combination remains failed and uncertain,
with fresh observation context and the original error retained in its cause
chain. Existing `move_directory` callers keep their erased result and behavior;
only provisioning receives the private classification needed for its policy.

Only bundled-runtime activation on Windows will consume the proven no-move
classification. It will require raw OS error 5, then repeat the same checked move
at 20-millisecond spacing until it succeeds or a two-second deadline expires.
Each failed attempt is freshly reconciled. A different error or any result other
than proven no-move ends recovery immediately. The two-second budget is a modest
implementation allowance, not a measured Windows guarantee.

The activation loop will remain in the same async provisioning future and use
an async timer for its 20-millisecond waits, avoiding blocking sleeps and a
detached publication task. That future retains one checked source `Directory`,
the `PrivateTemp`, and the cache lock through every attempt; success, terminal
failure, and caller cancellation all release authority through the same owned
future. The terminal error keeps the first typed activation error as the primary
cause and adds bounded retry/reconciliation context without masking it.

A test-only observation seam will signal classified attempts around the real
checked move without replacing it. The native held-descendant fixture can then
release its real blocker only after observing the first denied no-move result,
proving the production retry. Other controls keep the blocker, mutate names or
identities, and cancel the caller to prove the deadline and ownership boundaries.

## Risks / Trade-offs

- A two-second retry can delay reporting a persistent access denial. → Only the
  exact access-denied plus proven no-move state enters the loop; all other states
  stop immediately, and the total recovery period is fixed.
- A name can change between observations and the next move attempt. → Every
  attempt uses the retained source identity and the existing checked publication
  primitive, then reconciles again before another attempt or success claim.
- Cancellation can arrive between checked attempts. → No publication task is
  detached; dropping the same owned future releases its source, private stage
  and cache lock together, and native tests prove no partial destination or
  competing-writer window.
