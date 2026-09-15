## Why

Fresh sessions with equal activation currently fall through to ascending UUID
order. Those IDs are hashes of role/name seeds, so the first facing identity is
an implementation accident rather than an authored mode choice, even though the
event calls it stable identity.

## What Changes

- Use each built-in framework's existing authored part order only when targeting,
  active focus, maximum activation and previous-speaker continuity leave a tie.
- Skip ineligible authored identities, then use stable ID order only when no
  authored tied candidate remains.
- Emit `mode-authored-order` or `stable-id-order` as the actual fallback reason.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `peer-cognition`: Speaker selection uses the maintainer-approved authored cold
  order without changing authority or stronger selection evidence.

## Impact

`packages/kuru-core/src/framework.rs` exposes the built-in order as policy and
`packages/kuru-runtime/src/engine.rs` consumes it. Runtime selection tests and
architecture documentation change; no persisted identity or configuration does.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
