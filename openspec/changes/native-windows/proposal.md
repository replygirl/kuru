## Why

Kuru's first release omitted native Windows even though portability is expected
of the harness. The user explicitly requires native Windows CI and installation,
with runtime dependencies embedded so installing Kuru is sufficient on every
supported platform.

## What Changes

- Add native Windows 10 version 1809 or newer x64/MSVC application support,
  required Windows CI, and a Windows release archive alongside the existing
  macOS/Linux targets, without a separate MSVC redistributable installation.
- Put reused OS primitives behind a small Rust package boundary: safe file
  identity/privacy/reparse checks, durable replacement and owned process trees.
  Confine any necessary audited unsafe Windows API calls to that boundary.
- Port the Dolt supervisor, provisioning and migration, CLI defaults, command
  execution, file/shell tools, installer and updater without weakening their
  existing ownership, isolation or recovery guarantees.
- Add native Windows process, database, protocol, ConPTY, install and update
  tests. Preserve the current workspace coverage threshold and Unix tests.
- Extract the embedded full Dolt payload internally on first use, without a
  runtime download or a separately installed database. The companion
  embedded-runtime change supplies the verified payload with each application
  build; this change adds its Windows target.

## Capabilities

### New Capabilities

- `native-windows`: native Windows application behavior, OS boundaries and
  meaningful platform verification.

### Modified Capabilities

- `repository-delivery`: Windows binary/mise/PowerShell installation, platform
  release assets and safe running-executable updates.

## Impact

Affected packages: new `packages/kuru-platform`; `kuru-memory`, `kuru-connectors`,
`kuru-delivery`, `kuru-runtime`, and `apps/kuru-tui` plus CI/release workflows and
documentation. Platform primitives remain separate from cognition, provider,
storage and packaging policy. No change to peer roles, memory namespaces or the
Dolt schema is intended. Existing published release assets stay immutable.

## Surfaces

- [x] interactive — CLI/TUI defaults and native terminal behavior
- [x] deploy — Windows CI/release jobs and local process topology
- [x] integration — Windows APIs, Dolt ZIP, native process and filesystem contracts
- [x] agent-behavior — native shell and command execution preserve tool authority
