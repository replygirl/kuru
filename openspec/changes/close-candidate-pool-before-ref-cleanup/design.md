## Context

A candidate owns a main `live` view and a separate branch `view`. In the managed owner, the attachment retains that candidate after dispatching promotion, so the branch pool still has a Dolt session when successful transition cleanup deletes or renames candidate refs. Windows native coverage observed Dolt refusing the cleanup as a branch in use.

The shared candidate write guard already serializes transition against candidate writes and uncertain-operation recovery. A rejected or conflicting transition has not resolved the candidate and must keep its view usable.

## Goals / Non-Goals

**Goals:**

- Drain only the candidate branch pool after a transition result is durable and before ref cleanup.
- Preserve the live main pool, exact receipts, bounded waits and candidate usability on rejection or conflict.
- Surface a pool-close failure and retain refs rather than force deletion.

**Non-Goals:**

- Change Dolt branch commands, use force deletion, widen timeouts, or alter service/process lifetime.
- Change candidate selection, promotion proof, undo, recovery or public APIs.

## Decisions

- Retire the candidate branch through the shared server registry under the existing write guard. The registry removes the branch entry and closes that exact shared pool under its existing close grace; direct pool closure would leave a cached closed entry that a later inspection could incorrectly reacquire.
- Rejected and conflicting promotion paths return before pool retirement and retain a usable candidate. Once exact transition preflight succeeds, registry retirement fences the branch before status-ref mutation; after the main merge commits, retirement is repeated idempotently before cleanup so recovery also covers already-promoted targets.
- Record the in-memory promoted result only after retirement and ref cleanup both succeed. A post-commit cleanup error therefore retains the service handle and an exact retry re-enters the committed-target path to finish cleanup before returning success.

## Risks / Trade-offs

- A retained clone of a successfully resolved candidate becomes unusable. This matches the resolved-candidate contract and is covered explicitly.
- A retirement or cleanup failure can leave a durable transition marker. Existing exact transition recovery handles that retained state; the change does not infer rollback.
