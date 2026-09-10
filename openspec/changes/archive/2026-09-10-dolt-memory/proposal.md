## Why

Private memories and dreaming need durable version history and isolated candidate
changes. Replace SQLite as the live store now, using full Dolt while the data and
deployment surface are small, and preserve existing conversations during migration.

## What Changes

- Add package-owned managed Dolt installation, an authenticated local SQL sidecar,
  async memory access, revision inspection and lossless legacy import.
- Keep project and part namespaces isolated; pin each memory view to its branch.
- Stage complete dreams on a candidate branch and promote validated results from
  their expected base. Keep undo compensating so later conversations survive.
- Provide small memory status/history commands and clear startup/recovery errors.
- Update CLI/runtime integration, actual database tests, mise/CI tasks and docs.
- BREAKING: move the Rust MemoryStore API out of kuru-core and make storage I/O
  asynchronous. Dolt replaces the normal SQLite database; SQLite is retained only
  as a read-only import dependency for existing data.

## Capabilities

### New Capabilities

- `versioned-memory`: managed full-Dolt lifecycle, private async storage, revisions,
  candidate branches, recovery and legacy migration.

### Modified Capabilities

- `peer-cognition`: whole-dream isolation, durable promotion and compensating undo.
- `repository-delivery`: managed runtime provisioning and real Dolt quality gates.

## Impact

New packages/kuru-memory; core configuration/contracts; runtime actors, persistence
and dreaming; TUI/CLI lifecycle and tests; package-owned mise tasks and CI coverage;
installation, memory and development documentation. Runtime downloads use verified
pinned official assets and require no compiler or manual Dolt server command.
No DoltHub, hosted database, generic backend framework, extra model supervisor,
release archive format change, new language toolchain or Pages trigger is added.
Existing SQLite files remain preserved; implementation tests use isolated data.

## Surfaces

- [x] interactive — startup progress/errors, memory status/history and existing controls
- [x] deploy — managed native runtime, owned subprocess lifecycle and CI fixtures
- [x] integration — full Dolt MySQL protocol, revisions and SQLite import
- [x] agent-behavior — persistence isolation during dreaming; prompts and peer roles stay unchanged
