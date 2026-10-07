# Proposal

## Why

Completed standalone `kuru run` commands currently retire their checked memory
service on the last detach. Sequential scripts therefore restart Dolt for every
request despite the service architecture, leaving roadmap O3 incomplete.

## What Changes

- Retain an unused checked service for a bounded idle interval after its starter
  has attached and all attachment tasks have settled.
- Add `memory.service_idle_timeout_secs`, default 30, range 0 through 300; zero
  explicitly selects immediate retirement for isolated lifecycle fixtures.
- Preserve active-client ownership, startup deadlines, maintenance retirement,
  authenticated generation checks and endpoint retirement before reap and release.
- Prove sequential ordinary CLI calls reuse the same service and engine and
  that idle expiry eventually closes both without retaining fixture resources.

## Capabilities

### New Capabilities

### Modified Capabilities

- `project-memory-service`: bounded idle retention and sequential process reuse.
- `configuration-schema`: idle-retention configuration parser/schema parity.

## Impact

Memory-owned service policy and internal launch arguments; core memory config and
its published schema; focused service/CLI fixture configuration and docs. No new
provider transport, storage schema, dependency or session control API is needed.

## Surfaces

- [x] interactive — user-configurable scripted CLI reuse
- [x] deploy — checked native service lifetime
- [x] integration — published configuration schema
- [ ] agent-behavior — prompts and inference ownership remain unchanged
