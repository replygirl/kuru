## Why

Main CI reports RustSec advisory `RUSTSEC-2026-0285` against locked `rustls` 0.23.44. The compatible patched release is 0.23.45.

## What Changes

- Update only the resolved `rustls` entry and required checksum in `Cargo.lock` to 0.23.45.
- Verify the advisory is absent and the affected connector transport graph still builds and passes its focused tests.

## Impact

Cargo resolves the patched compatible Rustls release for existing TLS consumers. No manifest constraint, application source, public behavior, workflow, or dependency family changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
