## MODIFIED Requirements

### Requirement: Expressive responsive terminal presentation

The terminal interface SHALL provide a coherent visual identity with distinct
framework scenes, readable conversation and a unified composer pairing model,
effort and framework values with their selector shortcuts. It SHALL avoid redundant
panel labels, preserve editing/cancellation on narrow screens, and distinguish
decorative geometry from factual actor activity and relationship membership.

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

### Requirement: Bounded and optional motion

The terminal interface SHALL enable restrained ambient animation by default and
respond to editing with bounded, decaying visual feedback. Ambient presentation
SHALL NOT invent actor work, message delivery or modeled emotional state. Idle
animation SHALL run at no more than four frames per second and interactive/work
animation at no more than 12.5 frames per second. The existing startup reduced-motion
environment override SHALL preserve static equivalents without occupying normal
control space with a motion toggle.

#### Scenario: Settled idle interface
- **WHEN** the interface is idle and then the user edits or pastes input
- **THEN** a quiet ambient scene responds briefly to that input, while text, caret
  and actor state remain stable and animation returns to its idle cadence.

#### Scenario: Reduced motion
- **WHEN** KURU_REDUCED_MOTION=1 is set at launch
- **THEN** ambient and input animations remain static while all meaningful state and
  input updates remain available.

## ADDED Requirements

### Requirement: Durable interactive project preferences

Interactive mode and provider-specific model/effort choices SHALL persist outside
tool roots for the canonical project directory and survive a fresh process launch.
Remembered interactive choices SHALL override user/ancestor project defaults;
explicit local configuration and CLI flags SHALL override those choices for that
invocation without silently replacing them. Model/effort choices SHALL remain a
consistent provider-specific pair, including an explicitly cleared effort. Preference
writes SHALL be atomic and report failure without partially changing live state.

#### Scenario: Quit and relaunch
- **WHEN** a user changes mode, model or effort, quits and launches again in the same
  directory using the same data store
- **THEN** the choices are restored in a new conversation without requiring --resume.

#### Scenario: Temporary override and project isolation
- **WHEN** a remembered selection is overridden explicitly for one launch or another
  project is opened
- **THEN** the override takes effect only for that invocation and each project's
  remembered choices remain intact.

#### Scenario: Existing session and failed write
- **WHEN** an existing session is resumed or a preference write fails
- **THEN** resumption keeps that session's framework without rewriting preferences,
  and failed writes leave both live choices and saved values consistent.
