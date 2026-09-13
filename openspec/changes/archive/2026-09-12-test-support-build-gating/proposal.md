## Why

Ordinary memory builds currently compile public real-engine fixture support and
the parent fixture binary. That test scaffolding is not part of a release build
and can obscure the intended prepared-bundle failure path.

## What Changes

- Gate memory test support and fixture binaries behind the existing
  `test-support` Cargo feature, enabling it for tests and coverage consumers.
- Keep ordinary TUI, connector, platform, and memory build/release tasks off
  their existing `test-support` features.
- Preserve the existing prepared-bundle error and package-owned mise workflow.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

<!-- None. -->

## Impact

- Memory, TUI, connector, and platform build-task feature selection; memory
  fixture exports/binary and the runtime/TUI test manifests that consume them.
  No production runtime behavior or dependency changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
