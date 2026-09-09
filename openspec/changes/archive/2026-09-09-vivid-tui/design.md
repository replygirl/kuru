## Context

The application uses Ratatui and Crossterm. Its View contains public conversation,
active part identities and runtime events; the current loop repaints unconditionally
and the current sidebar exposes raw event strings. Existing key bindings, Unicode
editing and asynchronous cancellation must survive the visual update.

## Goals / Non-Goals

**Goals:** Make the pool legible as an equal-peer system, isolate visual code from
command execution, and keep rendering predictable and inexpensive.

**Non-Goals:** Provider changes, new model prompts, storage migrations, images,
Nerd Font requirements, or a copy of another product's brand.

## Decisions

- Use an ink palette with mint, lilac, amber and blue accents, rounded panels and
  generous message spacing. OMP's semantic color and loader treatment are references;
  copying its entire theme engine adds unrelated configuration scope.
- Extract rendering into a terminal-only module. Derive peer state, route highlights
  and relationships from existing events/topology. Random activity was rejected
  because the constellation should explain what the runtime is doing.
- Render animation at no more than 12.5 frames per second only during work or a
  finite welcome sequence. Repaint on input/events otherwise; an always-running
  animation timer would waste CPU across multiple terminal panes.
- Offer F6 and KURU_REDUCED_MOTION=1, with text labels accompanying color. Use
  existing dependencies and standard glyphs rather than requiring icon fonts.
- Keep the composer on small screens; progressively hide decorative content and
  collapse the pool sidebar. TestBackend and a real PTY exercise the same renderer.

## Risks / Trade-offs

- [Dense peer graph at small sizes] → Show a compact membership list and only draw
  a constellation where there is sufficient space.
- [Animation makes latency distracting] → Animate small activity accents; leave
  transcript content static and allow motion to be disabled.
- [Terminal palette variation] → Use explicit high-contrast colors and non-color labels.
- [Long conversation wrapping] → Exercise narrow screens, multiline input, Unicode
  and scrolling in behavioral rendering tests and PTY checks.

## Operational surface

This is the existing local Rust binary and terminal surface on supported macOS
and Linux architectures. It adds no bind address, secrets, remote services or
connection limits. Runtime launch remains `mise run run`; F6 toggles motion and
KURU_REDUCED_MOTION=1 chooses a static startup. Existing provider authentication,
permission flags and terminal restoration remain in effect.
