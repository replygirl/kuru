# Proposal

## Why

The current TUI uses fixed RGB styling for every terminal and does not honor
`NO_COLOR`. Phase 2 parity requires configurable semantic themes and readable
limited-color presentation while preserving permission, error and status meaning.

## What Changes

- Add dark and light presentation themes with a finite validated map of named
  semantic role colors through the existing configuration layers and schema.
  Preserve the current dark palette as the default.
- Resolve one invocation-local palette from the configured theme, nonempty
  `NO_COLOR` and the terminal's advertised truecolor, 256-color or 16-color
  capability; use plain styling for unknown or monochrome terminals.
- Apply roles to the current transcript, cards, composer, dialogs, status and
  scene without changing their controls, content or geometry. Retain meaning in
  labels and symbols when color is absent.
- Keep structured and redirected CLI output plain. Cover actual emitted terminal
  sequences, configuration precedence/validation and practical narrow/wide frames.

## Capabilities

### New Capabilities

- `terminal-themes`: Semantic palettes and terminal color negotiation that retain
  textual meaning at each supported depth.

### Modified Capabilities

- `configuration-schema`: Strict layered presentation settings and matching
  published schema.

## Impact

Owning core configuration/schema, TUI presentation and terminal startup, CLI
human-output policy, a read-only Windows console-capability query behind the
existing platform boundary, docs and focused configuration/frame/PTY fixtures. Recover
the relevant design and code from P17 reference commit `794caabc` against the
current integrated frontend; retain prior archives and unrelated source. No new
dependency, tool version, mise configuration, storage migration, provider or
framework behavior is needed.

## Surfaces

- [x] interactive — TUI theme/color presentation and CLI human-output policy
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
