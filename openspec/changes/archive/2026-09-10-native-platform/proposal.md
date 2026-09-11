## Why

Kuru needs a small, independently testable native OS boundary before its product
consumers can support Windows without weakening privacy or process ownership.
Building that boundary with compiled Rust fixtures allows real Windows evidence
without coupling its implementation to database, embedding or UI work.

## What Changes

- Add `packages/kuru-platform` with checked private filesystem creation,
  held-handle identity and reconciliable durable publication operations.
- Add Windows explicit native spawn descriptions, atomic owned process trees
  and cancellable private local IPC; shared filesystem primitives also have
  Unix implementations without changing existing consumer process behavior.
- Confine necessary audited Windows FFI to the package; retain safe public APIs
  and the unsafe-code prohibition in all consumer crates.
- Add package-owned mise tasks, compiled native fixtures and a required
  package-only Windows CI job. No Kuru product Windows support is claimed yet.
- Make the product `native-windows` change consume this foundation while keeping
  its Dolt-memory and embedded-runtime hard dependencies.

## Capabilities

### New Capabilities

- `native-platform`: safe owned filesystem, process and IPC primitives with
  meaningful native verification independent of application consumers.

### Modified Capabilities

None.

## Impact

New Rust package, workspace dependency/task registration and package-only native
CI wiring. No consumer adoption, database schema, migration, bundle preparation,
archive decoder, updater policy or application UI change. The platform package
does not depend on memory, delivery, connectors, runtime or TUI packages.

## Surfaces

- [ ] interactive — no product command or UI changes
- [x] deploy — native package-only Windows CI and owned fixture processes
- [x] integration — native OS filesystem, process and IPC contracts
- [ ] agent-behavior — no tool authority or model behavior changes
