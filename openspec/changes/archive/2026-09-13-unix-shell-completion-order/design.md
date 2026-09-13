## Context

`WorkerFinish::complete` sends through the caller oneshot before `Drop` removes
the completed owner. The receiver may immediately observe a stale reservation.
The pre-spawn panic drop path has the same report-before-removal ordering.

## Goals / Non-Goals

**Goals:**
- Remove only the completed worker ID before waking its receiver.
- Retain spawned workers whose cleanup remains unconfirmed.
- Prove the wake-time registry state without a timing loop.

**Non-Goals:**
- Change Unix process deadlines, cleanup confirmation, or registry structure.
- Change Windows shell behavior.

## Decisions

- Confirm completion, remove the worker's ID, then send the result. This uses
  the existing checked registry removal path and keeps a concurrent owner.
- Use a synchronous custom `Wake` in the oneshot receiver to inspect owner
  count at publication. It observes the ordering directly instead of waiting
  for a worker thread to drop.

## Risks / Trade-offs

- A cleanup error must not remove a spawned owner. The existing `report` path
  remains unchanged for unconfirmed cleanup, and the regression retains a
  second owner to catch broad removal.
