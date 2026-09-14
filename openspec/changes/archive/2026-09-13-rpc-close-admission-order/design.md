## Context

The RPC worker marks cleanup confirmed and sends its first close reply while its
command receiver still admits messages. The reply can synchronously wake a caller
that queues another close before the worker returns and drops the receiver, so the
new command is accepted but never serviced.

## Goals / Non-Goals

**Goals:**

- Make successful close publication the linearization point after admission has
  closed and cleanup has been confirmed.
- Resolve caller-side reply races from the existing authoritative completion
  state.

**Non-Goals:**

- Change cleanup deadlines, subprocess ownership, termination or retention.
- Add retries, sessions, registries or a second completion mechanism.

## Decisions

- Close the Tokio receiver at the start of `Command::Close`, before cleanup or
  reply publication. This prevents commands accepted after close begins without
  changing how already-admitted commands or peer ownership are handled.
- When the bounded reply wait expires, consult `Completion::result()` just as the
  existing send-failed and canceled-reply paths do. This returns success only for
  the worker's release/acquire-confirmed cleanup; false remains an error.
- Register a custom waker on the first close reply before sending the command and
  observe receiver admission plus completion inside that wake. This makes the old
  ordering fail deterministically without relying on scheduling delays.

## Risks / Trade-offs

- Closing admission rejects commands queued concurrently with close. Close is a
  terminal command, so accepting work that cannot be serviced is the invalid
  behavior this change removes.
- A reply timeout can return confirmed success after the reply itself is delayed.
  The result remains grounded in the existing authoritative completion flag and
  cannot turn uncertain cleanup into success.

## Integration contract

MCP wire behavior and transport timeouts remain unchanged. The change only orders
the local command-admission boundary before confirmed cleanup publication.
