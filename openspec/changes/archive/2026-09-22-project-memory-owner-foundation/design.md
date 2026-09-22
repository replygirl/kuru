## Context

The existing local `MemoryStore` already owns SQL pools, short write serialization, durable receipts and a Dolt lifetime-pipe supervisor. The archived private IPC primitives provide checked Unix sockets and Windows named pipes. The dependent `project-memory-service` change will route the public store facade and ordinary CLI through this owner; this change establishes only the internal process and transport boundary.

## Goals / Non-Goals

**Goals:** Make one owner process independently startable, attachable and safely retired; establish a typed protocol that can be extended behind the existing public store facade.

**Non-Goals:** Switch ordinary CLI opens, expose concurrent conversation admission, carry arbitrary SQL, implement P28 durable client-request deduplication, or move provider/runtime control into the service.

## Decisions

### Independent start and owner locks

The starter takes the short start lock, probes for an authenticated endpoint, then checks that owner authority is free before launching the same verified executable in an internal service mode. The child takes the distinct owner lock before opening `MemoryStore`; its published private endpoint and successful handshake are the readiness signal. The starter releases start authority only after attachment. This avoids the impossible pattern where a parent retains the lock the child must acquire. If the starter dies, the service can continue; a successor waits for the held owner or its reaped lifecycle before another launch. Rejected alternative: PID or occupied-port takeover.

### Native endpoint and complete identity

On Unix, the socket sits under a checked short owner-private `/tmp/kuru-service-<uid>/<scope-prefix>` locator because long configured state paths exceed macOS socket limits. The prefix routes only; a handshake checks the complete canonical project bytes, scope, store instance, generation, secret, protocol and schema. Windows uses the existing DACL-restricted named pipe. The endpoint record is private discovery, not an ownership grant. Rejected alternative: loopback TCP or a hash prefix as authentication.

### Typed connection and bounded memory

Each authenticated connection is a live attachment and can carry enumerated calls. Separate handlers allow reads to overlap while existing store write locks serialize short mutations. The 16 MiB typed-message limit can expand to roughly six times its size when JSON escapes controls, so frames are capped at 100 MiB and a 128 MiB shared semaphore bounds simultaneous request-frame memory. At most 32 connections are attached. Request IDs and generation are checked on replies; a failed exchange closes the attachment and never automatically replays a write. Existing durable store receipts still reconcile its SQL uncertainty; client-request deduplication belongs to P28. Rejected alternative: SQL-shaped RPC or one mutex around entire turns.

### Explicit owner shutdown

An attachment waits without a read deadline until its next first byte; a partial frame has a 35-second deadline. The owner starts a 30-second idle timer only when no attachment task remains. Its normal close drops the listener, closes store pools, awaits supervisor/Dolt reap, retires only its exact endpoint generation and releases its retained owner lock. A crash leaves the existing lifecycle lease and supervisor to enforce reap before any new engine opens. Rejected alternative: lockfile unlinking or process killing based on stale observations.

## Operational surface

The service starts on demand as an internal mode of the same native executable as its client, with the target's verified embedded full-Dolt archive; there is no container, daemon installer or network bind address. Unix uses the checked short private socket directory; Windows uses a private named pipe. The endpoint record contains only a fresh generation secret in owner-private state, not a provider or persistent Dolt credential. Up to 32 connections can attach and the shared request-frame budget is 128 MiB. An already-running older binary must pass the protocol/schema handshake; the internal entry alone does not make normal CLI commands attach yet. Native macOS, Linux and Windows builds each use their own bundled engine architecture.

## Integration contract

`kuru-platform` owns the socket/pipe and checked native directory mechanics. `kuru-memory` owns project and store identity, the endpoint record, typed operation inventory, SQL pools and supervisor lifecycle. The internal `kuru`/`kuru-memory` entry only dispatches to that service; runtime and CLI keep the existing local `MemoryStore` until the dependent facade migration. The protocol carries UUID request IDs, exact native canonical project bytes, a SHA256 project scope, a physical store instance, a fresh service generation and integer schema version as separate fields. The fixture uses the same bundled executable service entry and real Dolt; no SDK or external database protocol is added for clients.

## Risks / Trade-offs

- **[Cold startup and resident idle memory]** → Process spawn and handshake add cold latency; reuse one warm engine across sequential attachments and stop it after 30 idle seconds. The current exclusive conversation lease already prevents simultaneous ordinary engines, so this is not a claimed current-state consolidation speedup.
- **[Large JSON frame memory]** → Keep a fixed frame cap, 128 MiB request-byte budget and attachment limit; later facade work must page history/export rather than loop over row RPCs.
- **[Incomplete public API in this boundary]** → Keep ordinary `MemoryStore` local and the service internal until the dependent facade and CLI integration are verified; do not advertise this as concurrent-session support.
- **[Native process differences]** → Use platform private IPC and Windows owned-process creation; require hosted native Windows/Linux checks before merging, never infer execution from cross-compilation.
