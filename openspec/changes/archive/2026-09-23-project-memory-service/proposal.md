## Why

One conversation process currently holds the project writer lease and the lifetime of its Dolt supervisor. A second process must wait or fail, and every separate `kuru run` restarts the engine. Phase 2 needs a per-project storage owner whose lifetime is independent of any conversation, while preserving the existing recovery and private-history guarantees as concurrent sessions are introduced in later changes.

## What Changes

- Start and attach to one private per-project memory service on demand. The service retains the owned Dolt supervisor, project storage handles and lifecycle authority until all clients and in-flight operations have drained and a documented idle interval has elapsed.
- Give clients a versioned, bounded local handshake that verifies the canonical project and store instance. An old or incompatible active service produces an actionable error; no client uses a PID, occupied port or stale endpoint as takeover authority.
- Route storage access through a service-owned client boundary and move clean shutdown and crash recovery to that owner. A lost client cannot close another client's store or engine; the dependent `durable-service-operation-receipts` change supplies request-bound uncertain-write reconciliation before ordinary managed writes activate.
- Expose a bounded checked inventory and status for retained dream candidate refs, with explicit exact-head abandonment through the existing memory CLI and registered TUI command surface. A moved live base is reported as a conflict; neither inspection nor an unresolved transition silently promotes, replays or abandons a candidate.
- Keep the existing exclusive conversation-driver lease during this change. P28 proves cross-process mutation serialization and the existing Dolt `AUTO_INCREMENT` sequence; session-private history and concurrent conversation admission follow in P29/P30. The service can host multiple attached storage clients, including inspection, without admitting two ordinary conversation drivers yet.

## Capabilities

### New Capabilities

- `project-memory-service`: Per-project service discovery, client admission, compatibility and independent idle lifetime.

### Modified Capabilities

- `memory-store-lifecycle`: Shared-store close and uncertain-operation recovery move to the service owner.
- `versioned-memory`: Owned Dolt lifetime and inspection attachment work through the service while preserving migration and lifecycle safety.

## Impact

- `packages/kuru-memory` extends the archived internal owner foundation with a public store facade and private typed client, including candidate, usage-ledger and export handles; `packages/kuru-platform` retains native IPC/process mechanics.
- `apps/kuru-tui` attaches ordinary memory opens to the owner under the retained conversation lease and routes exact-ref candidate inspection and explicit abandonment through its existing memory commands and shared TUI command registry; the already-packaged internal service entry remains private. No public control API, provider loop or credential-store access enters the service.
- This service change adds no memory schema migration itself; its managed-write integration requires the separate retained-receipt migration. The service protocol has its own explicit version and bounded frames, independent of Dolt schema and activation formats.
- Existing exclusive conversation-driver behavior remains until P29/P30; storage service and engine reuse are visible across sequential `kuru run` processes.

## Surfaces

- [x] interactive — startup progress and incompatible-service diagnostics in CLI/TUI
- [x] deploy — service lifecycle and packaged native runtime behavior on each supported OS
- [x] integration — private local IPC and compatibility handshake
- [ ] agent-behavior — no model, tool or prompt changes
