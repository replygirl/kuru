# Proposal

## Why

The pinned model metadata lacks GPT-6.1 Sol and GPT-6 Luna even though live discovery already accepts new model IDs. Successful ChatGPT login does not enumerate available models, so users cannot see their current choices until a separate command or startup.

## What Changes

- Add sourced metadata for `gpt-6.1-sol` and `gpt-6-luna`, preserving older exact model IDs and provider-advertised facts.
- Enumerate the fixed native subscription catalog after either successful login flow. Catalog failure preserves successful authentication and reports safe retry guidance.
- Preserve automatic fresh discovery on every open and explicit `models` invocation, including unknown model IDs and efforts. No persistent daily cache is needed for this existing behavior.
- Document exact model selection: Kuru has no `astra`, `sol`, `terra`, or `luna` shorthand aliases to retarget.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `model-catalog`: current sourced metadata, post-login enumeration, and automatic discovery without requiring application updates.

## Impact

`packages/kuru-core/assets/model-catalog.v1.json`, model catalog tests, connector-owned subscription discovery and fixtures, `apps/kuru-tui/src/authentication.rs`, login presentation tests, and user documentation. No new dependencies, migrations, persistent cache, aliases, mise changes, roadmap HTML changes, releases, or paid inference.

## Surfaces

- [x] interactive — login lists models and reports catalog unavailability
- [ ] deploy — no deployment topology changes
- [x] integration — native subscription catalog discovery
- [x] agent-behavior — exact model metadata and preserved provider discovery
