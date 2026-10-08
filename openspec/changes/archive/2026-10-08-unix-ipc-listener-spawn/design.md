# Design

## Context

Mio creates Darwin Unix sockets before separately applying close-on-exec. The
private listener currently binds outside the lock used for owned descriptor
creation and child creation. Outgoing and accepted sockets have the same Darwin
creation window. Native CI observed a stale fixture listener still
accepting connections, without capturing the historical inheriting process.

## Decisions

Use the existing platform spawn lock during synchronous listener binding and
UnixSocket creation, and during each poll of the pinned existing accept future.
Release it before an asynchronous wait or Pending return; retaining the existing
accept future preserves its per-waiter readiness semantics. Keep the binding
helper private and the lock crate-visible. Existing checked directory, socket identity, permission, and
caller-authentication contracts remain in their current owners.

## Operational surface

The endpoint remains an owner-private Unix socket under the checked short
service directory, with the existing 0600 socket mode and generation name.
There is no TCP bind address, container, new connection limit, or required
secret. Native macOS and Linux runners exercise the affected implementation;
Windows IPC and all binary/tool versions and target architectures are unchanged.

## Risks / Trade-offs

Listener creation can briefly wait behind an owned spawn. No child I/O, runtime
await, caller callback, or process reap occurs under this lock. Unrelated legacy
spawns and unrelated non-atomic descriptor creation retain their documented
limitations; this correction covers platform-owned private Unix IPC sockets.
