## Why

Phase 0's implemented cold-facing rule is absent from the architecture and framework documentation even though its archived change record claims that documentation changed. The shared mode-0700 remedy also reaches ordinary and legacy store opens but bypasses other memory-owned private directories, so an owner-owned Unix directory with unsafe mode bits can still fail with a bare permission error at Dolt provisioning or server startup.

## What Changes

- Document the built-in authored cold-facing order, the `mode-authored-order` and `stable-id-order` reasons, its position after stronger selection rules, and its no-authority guarantee.
- Route memory-owned private-directory validation through one memory-package diagnostic helper so Dolt cache, versions, cold-probe, installation, and server-owned directories receive the existing exact-path remedy when eligible.
- Preserve refusal, checked ownership and identity, no automatic permission changes, no Unix mode guidance for links or foreign-owned paths, and the existing Windows owner-privacy guidance.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Documentation changes in `docs/architecture.md` and `apps/kuru-docs/concepts/frameworks.md`. Implementation and regression coverage remain within `packages/kuru-memory`, primarily its shared filesystem helper and the provisioning/server call sites that need a checked directory handle; no persisted format, platform policy, runtime speaker behavior, dependency, or migration changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
