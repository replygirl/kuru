# Design

## Context

Dream participants share a connection-owned candidate attachment. A held provider request proves one participant reached inference, while another can still await a context read wrapped in cancellation.wait. Cancelling that sent read drops its exchange, leaving no complete stream and no mutation receipt. Exact candidate reconnection correctly refuses; automatic shutdown abandonment consequently fails its revision read.

## Goals / Non-Goals

Preserve the sent read until its response, then permit existing exact cleanup and typed cancellation. Do not reconnect a lost candidate, replay writes, change actor/provider policy, or invent an outcome from missing evidence.

## Decisions

For candidate-only nonmutating calls, acquire the existing owned attachment mutex guard before spawning a finite response-draining task. The task owns the exact view, call and guard through the existing bounded checked exchange. Caller cancellation drops only the waiter. Candidate cleanup takes the same mutex and therefore follows the actual read response. Ordinary main reads and every mutating call retain their existing paths and receipt fences. No task retains a sender or its own JoinHandle; no idle task or persistent registration exists.

Use the existing sent-frame ReplyPause for a deterministic real-service read cancellation regression. After aborting the read caller, release the actual exchange and prove the same candidate remains readable and can be abandoned, with unchanged main revision and missing exact private ref. Keep the existing real CLI SIGINT/completed retry test unchanged.

## Risks / Trade-offs

Cleanup may wait for an already-sent read under the existing operation deadline. Transport failure still loses the attachment and is reported honestly; the no-reconnect guard and mutation uncertainty protections remain mandatory. This task owns a bounded read, not remote rollback or a process-lifetime cleanup framework.

## Operational surface

The existing private per-project service and connection limits remain unchanged. No new bind address, credential, binary pin or platform requirement is introduced. The finite task runs on the existing client runtime and holds one already-selected candidate attachment; its response remains bounded by the existing service exchange deadline. Tests use isolated temporary projects, fake provider data and the package-prepared supervisor under the existing native cache and FD4096 owning shell. Host behavior and Windows compilation are recorded separately from hosted native coverage.
