## Why

The existing memory supervisor is owned by one conversation process, so a later process cannot attach to the same live Dolt engine. Before changing runtime storage handles, Kuru needs a native, independently owned process with a verifiable project identity, bounded transport and race-free election. This foundation provides that internal boundary while the existing `MemoryStore` and ordinary CLI behavior remain in place.

## What Changes

- Add an internal service entry to the packaged executable and `kuru-memory` fixture binary. The service owns the existing writable `MemoryStore` and Dolt supervisor, then closes them before retiring its endpoint and owner lock.
- Elect one service with distinct short starter and retained owner locks. Authenticate attachments with a private native endpoint and full project, store, protocol, schema and generation identity; never infer ownership from an endpoint, PID or occupied port.
- Carry an initial set of typed, bounded storage operations across the local IPC connection. Keep an attachment alive through a turn, limit concurrent frames, and retire the service after a 30-second idle period.
- Verify native cold-start competition and real Dolt read/write/reap behavior without switching the public store facade or enabling concurrent conversation drivers. The dependent `project-memory-service` change owns complete facade coverage, CLI reuse and maintenance integration.

## Capabilities

### New Capabilities

- `project-memory-owner`: Internal native owner election, authenticated storage attachment and idle lifecycle.

### Modified Capabilities

## Impact

- `packages/kuru-memory/src/{service,main,server,store}.rs`, a typed service transport module, `packages/kuru-platform/src/local_ipc.rs`, and the internal entry dispatch in `apps/kuru-tui/src/main.rs`.
- No memory schema change, public CLI flag, provider loop, control API or dependency addition. Ordinary runtime calls still use their existing local `MemoryStore` until the dependent facade change is complete.

## Surfaces

- [ ] interactive — no normal user-facing command changes yet
- [x] deploy — a packaged internal process owns the engine when explicitly started
- [x] integration — private native IPC and the versioned storage protocol
- [ ] agent-behavior — no prompt, model, tool or actor changes
