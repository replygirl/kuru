# Proposal

## Why

The TUI composer currently edits UTF-8 text by Unicode scalar, so cursor motion and deletion can split visible grapheme clusters such as combining sequences and joined emoji. It also lacks bounded prompt recall and a compact way to review large pasted blocks, which makes longer drafts harder to edit while the canonical prompt must remain exact.

## What Changes

- Add grapheme-safe horizontal, line-local, whole-draft and vertical cursor motions, with a visible cursor across practical terminal sizes.
- Add bounded, in-memory prompt history scoped to the selected session and Ctrl-R search, preserving unsent drafts through cancellation and selection changes.
- Render large paste spans as expandable, removable chips while retaining their literal bytes and line breaks in the canonical draft; reject pastes exceeding 128 KiB atomically.
- Clamp transcript and composer scrolling after content and terminal-size changes while retaining existing picker and approval input priority.

## Capabilities

### New Capabilities
- `interactive-composer`: Grapheme-safe editing, bounded selected-session prompt recall, literal-preserving paste presentation, and clamped composer navigation.

### Modified Capabilities

## Impact

Changes are limited to the TUI composer state and rendering, focused TUI unit/render/PTY coverage, and the command and first-conversation user guides. The existing exact-pinned `unicode-segmentation` dependency is reused; there are no memory schema, runtime API, CLI grammar, or persistent prompt-history changes.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
