## Why

Root and ancestor instruction capture is complete, but instructions inside a project subtree are not yet encountered when an actor uses that subtree. A tool can therefore read or change a nested file without the actor receiving the applicable `AGENTS.md` or `CLAUDE.md`, and discovering those bytes after a planned mutation would be too late to guide that action.

## What Changes

- Discover checked nested instruction sources only for authorized, path-qualified tool targets, composing them with the invocation's already captured root and ancestor sources under the same import, identity, and aggregate limits.
- Review the resulting complete authority manifest before newly discovered instruction bytes enter a provider prompt. Use the existing foreground review surface for interactive turns; headless turns return a bounded trust-required result when no one-time or stored approval applies.
- Refresh the actor's instruction context after discovery. A mutating call planned before new instructions were available settles as a replan request without executing or replaying its effect; read-only calls may continue after review and expose their results only for authorized candidates.
- Keep tool permissions independent of instruction trust. Native search gathers a bounded authorized candidate set before nested capture and reports omitted candidates or instruction branches truthfully.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `project-instructions`: path-qualified nested discovery, bounded composition, and actor refresh.
- `workspace-trust`: review and exact-byte binding for newly encountered nested sources before prompt activation.
- `provider-tools`: candidate-scoped discovery after permission and before path effects or result exposure.

## Impact

`kuru-core` instruction capture and manifest derivation, `kuru-connectors` checked tool admission and native search, `kuru-runtime` tool continuation and instruction state, `kuru-tui` foreground review and CLI diagnostics, the private approval-record format, and configuration/trust documentation and focused fixtures. No database schema migration or dependency addition is expected.

## Surfaces

- [x] interactive — foreground trust review and replan notice
- [ ] deploy — no deployment topology change
- [ ] integration — no third-party contract change
- [x] agent-behavior — instruction context and tool continuation
