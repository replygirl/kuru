## Why

Typing currently launches expanding rings and traveling marks on the composer.
These reactive effects distract from composing and make the interface feel jittery.
Ambient glyph swapping adds unnecessary flicker even when no input is arriving.

## What Changes

Remove typing-triggered decoration and its faster animation cadence. Keep the
composer still and let existing framework contours carry a slow, subtle color
cycle independent of input; contour glyphs and positions remain fixed.

## Capabilities

### Modified Capabilities

- `chat-harness`: revise motion behavior to separate quiet ambient color from
  editing and preserve useful runtime activity indicators.

## Impact

TUI view state, scene and composer drawing, motion regressions and interface docs.
No provider, preference, mode selector behavior, dependency or storage changes.

## Surfaces

- [x] interactive — terminal motion and composition
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
