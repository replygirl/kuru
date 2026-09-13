## Why

Opening a project memory store can extract a verified embedded engine, validate a
large cached payload, probe its exact version, recover staging, import legacy
SQLite data, and open Dolt before the application can render its TUI. The
application currently gives no causally accurate indication of that work, and a
legacy data directory with unsafe Unix permissions fails without an actionable
remedy.

The existing verification remains necessary on every open. This feature makes
real startup work visible without weakening digests, version probes, ownership,
activation, or the authority of the returned open result.

## What Changes

- Add a fixed, bounded observed-memory-open interface while retaining silent
  `MemoryStore::open` for library consumers.
- Emit typed stages at actual project-lock, managed-cache, verification,
  extraction, version-probe, database-preparation, database-open, and final
  ready boundaries.
- Render fixed application-owned progress only on CLI stderr; preserve stdout,
  JSON, existing error handling, and TUI ownership.
- Make legacy SQLite permission refusal name the actual unsafe Unix directory
  and its owner-only remedy; retain Windows native-owner privacy guidance.
- Prove a real old SQLite layout imports while preserving its original files and
  that cold, warm, corrupt, cancellation, and provider-free paths report only
  truthful stages.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `versioned-memory`: observed open stages and actionable legacy import privacy
  refusal while preserving verified provisioning and durable migration behavior.
- `public-documentation`: document truthful startup progress and legacy privacy
  recovery guidance.

## Impact

- `packages/kuru-memory/src/{lib.rs,store.rs,provision.rs,migration.rs}` and
  focused memory fixtures add the observed-open API and exact emissions.
- `apps/kuru-tui/src/cli.rs` and CLI/PTTY fixtures drain observed opens and
  render bounded stderr progress.
- Memory/configuration documentation explains the stage semantics and legacy
  permissions remedy.
- No schema, dependency, provider, credential, or warm-verification optimization
  changes.

## Surfaces

- [x] interactive — CLI/TUI startup observes fixed stderr progress before the
  normal terminal session begins.
- [ ] deploy — no deployment topology changes.
- [ ] integration — no third-party contract changes.
- [ ] agent-behavior — no prompt, tool, routing, or model-output changes.
