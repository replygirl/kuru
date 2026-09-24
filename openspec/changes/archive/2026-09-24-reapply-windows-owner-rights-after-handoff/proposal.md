## Why

On elevated Windows runners, `TokenOwner` can differ from `TokenUser`. Native
evidence shows that assigning that exact default owner changes the staged
file's effective zero-mask OWNER RIGHTS ACE into an inherit-only ACE, so the
strict private-file validator correctly refuses the handoff before publication.

The prior pre-owner DACL reapplication cannot survive the owner mutation. The
staged file must regain the same effective protected policy after the owner is
assigned and before any source DACL copy or publication can continue.

## What Changes

- Reapply the existing protected TokenUser full-access plus effective zero-mask
  OWNER RIGHTS DACL through the retained `WRITE_DAC` staged handle immediately
  after exact owner assignment.
- Remove the ineffective pre-owner reapplication and retain the existing source
  owner allowlist, strict final privacy check, exact source/staged owner check,
  source DACL copy and hard pre-publication failure.
- Preserve native descriptor-shape diagnostics for hosted Windows verification.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

The correction is confined to `packages/kuru-platform/src/windows/security.rs`
and its Windows security fixtures. It changes no public API, principal set,
access mask, publication contract or non-Windows behavior.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
