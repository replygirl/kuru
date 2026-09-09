## ADDED Requirements

### Requirement: Expressive responsive terminal presentation

The terminal interface SHALL provide a coherent color hierarchy, distinct speaker
and role treatments, a welcoming empty state, readable conversation content and
discoverable selectors. It SHALL adapt to narrow panes without losing composition
or cancellation, and SHALL show active parts and available relationships using
runtime state rather than invented activity.

#### Scenario: Conversation in a split pane
- **WHEN** a terminal is resized between wide and narrow layouts
- **THEN** messages, the editor and essential controls remain usable and a wide
  layout exposes the pool and its relationship membership.

#### Scenario: Peer activity
- **WHEN** peers deliberate, exchange messages or become the speaking identity
- **THEN** the interface distinguishes the participating identities and their
  activity using both labels and color without displaying private histories.

### Requirement: Bounded and optional motion

The terminal interface SHALL animate activity at a bounded cadence, stop recurring
visual work when settled and idle, and provide a discoverable reduced-motion
toggle. Reduced motion SHALL preserve all meaningful state labels and controls.

#### Scenario: Reduced motion
- **WHEN** the user disables motion through the terminal control or startup environment
- **THEN** animation frames remain static while activity and input updates still render.

#### Scenario: Settled idle interface
- **WHEN** no operation, input or finite welcome animation requires a visual update
- **THEN** the event loop avoids recurring full-frame rendering.
