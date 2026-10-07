# Design

## Context

Current render and scene functions use literal RGB values. The accepted current
frontend already includes composer, transcript and tool-card behavior missing from
the old P17 reference. Reuse only relevant styling mechanics from `794caabc`,
adapting current call sites rather than replacing old frontend or archive files.

## Goals / Non-Goals

**Goals:** One finite, validated presentation contract and palette per view; real
terminal evidence for emitted depth and textual meaning, preserving the current
dark appearance and layout.

**Non-Goals:** Runtime settings UI, theme scripts/files, terminal query protocol,
keybinding customization, screen-reader mode, new dependencies or tool versions.

## Decisions

1. Core owns only typed `UiConfig` and its finite validated role map. The app owns
   colors, terminal negotiation and rendering. This keeps terminal mechanics out
   of domain configuration and avoids independent per-widget palettes.
2. Use dark/light built-ins and #RRGGBB role overrides, inheriting unspecified
   roles. A larger theme framework adds no Phase 2 outcome. User-selected low
   contrast remains their choice; shipped palettes and text labels are tested.
3. Capture terminal environment at invocation startup without changing global
   variables. Nonempty NO_COLOR wins. Pure fixtures may select a resolved palette
   directly; actual PTY tests use child environments.
4. Replace literal renderer/scene styling with semantic roles. Resolve ambient
   interpolation only within the selected supported depth. Preserve row text,
   symbol positions and current cache/viewport ownership.
5. Check the pinned backend's actual 16-color output. If it spells named colors
   as indexed SGR, use the existing bounded streaming-writer approach from P17 to
   translate only those color commands to classic SGR. Other escapes and text
   pass through unchanged, including fragmented writes.
6. Human CLI status may use the same palette on its exact capable terminal
   stream. On Windows use a small read-only query at the existing platform
   console boundary; never enable modes for a CLI status. Data serialization and
   redirected output stay on their existing plain paths.

## Risks / Trade-offs

- Limited palettes or custom choices reduce contrast: assert shipped essential
  roles remain distinct and retain labels/symbols in every depth.
- Styling can affect Unicode geometry or stale layout caches: verify completed
  frames and cursor positions at practical widths without changing layout keys.
- A writer can mishandle partial escape sequences: bound pending bytes and test
  fragmented/combined commands and non-color output equivalence.

## Operational surface

Only existing local terminal and human CLI presentation changes. There is no new
listener, network request, secret, engine, process topology, version or supported
architecture. Required native checks exercise output/restoration on supported CI
runners; local acceptance is reported separately from those native outcomes.
