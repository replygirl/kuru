## Why

Kuru's user documentation currently lives alongside engineering records in the repository. A public documentation site will make installation, framework behavior, and protocol reference discoverable without publishing internal verification history.

## What Changes

- Add a curated VitePress application at `apps/kuru-docs` with local search and machine-readable documentation.
- Present installation, everyday use, framework concepts, and configuration/protocol reference in Kuru's own accessible dark and light visual identity.
- Keep the site independently buildable; root tooling and Pages deployment are coordinated in the sibling delivery change.

## Capabilities

### New Capabilities

- `public-documentation`: Navigable, accessible, deliberately selected public product documentation.

### Modified Capabilities

None.

## Impact

Adds `apps/kuru-docs/**` and exact VitePress 2.0.0-alpha.20 and vitepress-plugin-llms 1.13.5 development dependencies. No runtime, storage, provider, or protocol changes. Root workspace/toolchain/deployment integration belongs to the coordinated delivery change.

## Surfaces

- [x] interactive — browser documentation, navigation, local search, and framework comparison
- [ ] deploy — site source is independent of the sibling deployment workflow
- [ ] integration — no new runtime contract
- [ ] agent-behavior — no inference behavior changes
