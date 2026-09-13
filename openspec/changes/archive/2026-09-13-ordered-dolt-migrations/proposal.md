## Why

Kuru currently rejects every persisted Dolt schema except version 1, so it
cannot evolve durable memory without rebuilding or abandoning existing project
history. The pinned-engine proof also shows that transactional DDL can survive
a pre-commit disconnect as a dirty working root, requiring schema changes to be
built on an isolated branch and published only after complete validation.

## What Changes

- Add an immutable ordered migration registry and committed per-version
  receipts, while keeping schema version distinct from activation, identity and
  supervisor protocol formats.
- Upgrade writable old-schema stores one version at a time on strictly named
  exact-base migration branches, preserving failed attempts and publishing only
  a clean validated commit through reconciled fast-forward promotion.
- Discover current-step pristine, ready, failed and unresolved attempts after
  process loss; classify retained branches for already applied steps through
  registered receipts and ancestry so later writes and migrations remain
  possible without resetting or deleting history.
- Keep accepted migration work alive through caller cancellation, SQL-session
  teardown, server reaping and startup-lock release. Apply the same current
  schema and receipt chain before activating fresh or imported staging stores.
- Retain version-aware read-only validation for old candidate branches and
  preserve every existing main/candidate ref, row and revision.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `versioned-memory`: add ordered schema evolution, isolated publication,
  interrupted-attempt recovery and legacy-branch inspection to managed Dolt
  memory.

## Impact

`packages/kuru-memory` gains private migration registry, validation, attempt
discovery and owned startup-worker code plus native persisted-v1, staging,
reply-loss, cancellation and candidate-preservation fixtures. Memory schema 2
adds a migration receipt table and one version-1-to-version-2 receipt; existing
format-1 `ready.json`, identity records, supervisor protocol and public memory
APIs remain compatible. No dependency, workflow, provider, runtime-turn or UI
policy change is required.

## Surfaces

- [ ] interactive — no command syntax or TUI behavior changes
- [ ] deploy — existing native jobs exercise the change without topology changes
- [x] integration — pinned Dolt schema, branch and transaction behavior
- [ ] agent-behavior — no prompt, routing, tool or model behavior changes
