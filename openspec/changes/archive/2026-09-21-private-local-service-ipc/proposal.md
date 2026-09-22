## Why

Phase 2 needs a local storage owner that accepts successive processes without giving them Dolt or process authority. The existing Windows private pipe rendezvous admits one known child, while Unix has no checked service-socket boundary in `kuru-platform`. These OS primitives should be independently reviewable before the memory service uses them.

## What Changes

- Add a checked, owner-private Unix socket listener and client connector under a retained private directory, with a unique name per service generation.
- Add a Windows named-pipe listener for successive local clients, preserving the existing single-child identity-bound rendezvous. A cancelled accept can safely resume its pending native operation.
- State the boundary honestly: native permissions exclude other users; the consuming service still must authenticate each client, and a same-user process that can read the private endpoint record is within the local account authority.
- Keep these primitives unused by ordinary Kuru behavior until the project-memory-service integration is complete.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `native-platform`: Reusable private local IPC mechanics for a service with successive clients.

## Impact

- `packages/kuru-platform/src/{local_ipc,lib}.rs`, `src/windows/pipe.rs`, their native tests and the package's Unix Tokio dependency declaration.
- No memory schema, provider, CLI, public protocol or user-facing behavior changes. The workspace-pinned Tokio version remains unchanged.

## Surfaces

- [ ] interactive — no user-visible flow changes
- [x] deploy — native runtime IPC mechanics on Unix and Windows
- [ ] integration — no external service/SDK contract
- [ ] agent-behavior — no prompt, tool or model changes
