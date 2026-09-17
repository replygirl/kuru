## MODIFIED Requirements

### Requirement: Expressive responsive terminal presentation

The terminal interface SHALL provide a coherent visual identity with distinct
framework scenes, readable conversation and a unified composer pairing model,
effort and framework values with their selector shortcuts. It SHALL avoid redundant
panel labels, preserve editing/cancellation on narrow screens, and distinguish
decorative geometry from factual actor activity and relationship membership.
The standing composer controls SHALL also carry, in the same frame, the last
prepared request's labelled context-use estimate with its window provenance and a
labelled session cost estimate, beside the permission-grant counts. Those figures
SHALL be the ones `/cost` and `/permissions` report. An unknown price SHALL read
as unknown, never as a zero charge, an invoice or a quota. A width too narrow for
the full row SHALL abbreviate its values rather than drop a control.

#### Scenario: Changing frameworks
- **WHEN** a user previews or selects IFS, polyvagal, Freudian or Jungian mode
- **THEN** the corresponding scene has a distinct geometry and accurately named
  members, and the selected value remains visible beside its control.

#### Scenario: Complete interaction states
- **WHEN** the interface moves through empty, composing, working, completed,
  cancelled or failed states, including in a narrow pane
- **THEN** the composer, current selections and meaningful operation feedback remain
  legible without changing draft content or cursor position.

#### Scenario: Conversation in a split pane
- **WHEN** a terminal is resized between wide and narrow layouts
- **THEN** messages, the editor and essential controls remain usable and a wide
  layout exposes the pool and its relationship membership.

#### Scenario: Peer activity
- **WHEN** peers deliberate, exchange messages or become the speaking identity
- **THEN** the interface distinguishes participating identities and their activity
  using labels and color without displaying private histories.

#### Scenario: One frame carries the session's operating facts
- **WHEN** a completed frame is inspected at a practical narrow or wide terminal
  width after a prepared request
- **THEN** model, effort, mode, session cost, context use and permission state are
  all present in that frame, the cost and permission values match `/cost` and
  `/permissions`, and an unknown price reads as unknown rather than as a zero
  charge or a quota.
