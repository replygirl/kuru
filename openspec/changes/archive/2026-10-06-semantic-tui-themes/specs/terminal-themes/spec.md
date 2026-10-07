# Spec Delta

## ADDED Requirements

### Requirement: Semantic presentation palettes

The TUI SHALL resolve one invocation-local palette from validated presentation
configuration. Its dark default MUST retain the existing RGB palette. A light
theme and bounded named role-color overrides SHALL cover backgrounds, surfaces,
text, muted text, accents, information, warnings, errors and ambient scene marks.
Changing styling MUST NOT change controls, operation identity, content, editor
cursor geometry, private history or permission decisions.

#### Scenario: Selected theme reaches current presentation
- **WHEN** a valid theme or role override is selected through existing configuration
- **THEN** transcript, tool cards, composer, dialogs, status and scene use that palette while preserving their textual content and geometry.

### Requirement: Color capability negotiation

Nonempty `NO_COLOR` SHALL disable decorative color regardless of theme. Otherwise
Kuru SHALL select truecolor from a compatible advertised terminal with truecolor
capability, 256-color from a 256-color terminal, or classic 16-color from a known
classic terminal; unknown, dumb and monochrome terminals SHALL receive plain
styling. Native Windows TUI startup MAY use its already-enabled VT output
capability. A 16-color path MUST emit classic SGR color codes instead of RGB or
indexed extended-color codes. Negotiation MUST NOT query a provider or mutate
process-global environment.

#### Scenario: Color disabled
- **WHEN** nonempty NO_COLOR is inherited alongside a truecolor terminal and selected theme
- **THEN** completed TUI frames retain labels, symbols and keyboard actions without decorative foreground or background colors.

#### Scenario: Limited terminal
- **WHEN** the actual output terminal advertises 256-color or classic 16-color support
- **THEN** emitted color sequences stay within that depth, including the current scene and permission/error/status surfaces.

### Requirement: Textual meaning and plain data output

Permission choices, selected items, activity, warnings and errors SHALL remain
understandable through text and symbols independently of color. Human CLI color
SHALL follow the same theme and NO_COLOR policy only on a capable terminal stream;
Windows CLI output MUST confirm VT processing for the exact stream before
emitting color. Redirected output and structured JSON or JSONL MUST remain plain
and retain their existing bytes and redaction boundaries.

#### Scenario: Practical narrow and wide frames
- **WHEN** permission, error, status and Unicode-width content are rendered at 80 and 120 columns with plain or limited-color styling
- **THEN** their meaning, visible choices and cursor/layout behavior remain available without color alone.

#### Scenario: Structured or redirected output
- **WHEN** a command emits structured data or a human stream is redirected
- **THEN** output contains no decorative color escapes regardless of configured theme or terminal environment.
