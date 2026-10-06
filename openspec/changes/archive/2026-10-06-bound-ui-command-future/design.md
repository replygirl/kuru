# Design

## Context

The Windows coverage job exits with `STATUS_STACK_OVERFLOW` in the existing real slash-command fixture. One private async command function contains runtime turn, retry, dream and reconciliation branches; its full state propagates into callers. The application entrypoint already heap-pins its dispatch operation.

## Decisions

Keep the existing controlled command signature and move its unchanged body into a private inner function. The entry boundary heap-pins and awaits that one operation, retaining the same borrowed Harness, cancellation token, reviews and return value. No task, thread, executor, timeout or stack-size setting changes.

## Risks / Trade-offs

One allocation per command bounds the state embedded in callers. Local macOS fixtures establish behavior preservation; only a successful corrected native Windows job establishes that the observed Windows overflow is resolved. Keep that distinction explicit through archive and delivery.

## Operational surface

The existing TUI command caller and supported native binaries remain unchanged. This fix adds no bind address, process, secret, connection limit or runtime dependency; it changes only where the private operation's state is retained.
