## Why

The current terminal interface exposes the peer pool but gives its activity little
visual hierarchy or personality. A richer, animated interface should make the
parts and relationships legible while keeping conversation and composing primary.

## What Changes

- Introduce a cohesive dark palette, colorful role accents, polished conversation
  presentation, a distinctive welcome screen, and clearer selectors and shortcuts.
- Visualize the live pool and relationships with restrained, bounded animation.
- Adapt to narrow panes and provide reduced motion without losing state cues.
- Preserve keyboard controls, cancellation, privacy and session behavior.

## Capabilities

### New Capabilities

### Modified Capabilities

- `chat-harness`: expressive, responsive terminal presentation and accessible motion.

## Impact

Changes are confined to the terminal application, behavioral rendering tests and
usage/design documentation. No storage migration or provider/protocol change is
required. Prefer existing Rust dependencies and ordinary terminal glyphs.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology
- [ ] integration — a third-party/external contract
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
