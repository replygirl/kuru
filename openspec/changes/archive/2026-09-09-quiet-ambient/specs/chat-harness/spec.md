## MODIFIED Requirements

### Requirement: Bounded and optional motion

The terminal SHALL enable quiet ambient color animation independent of input.
Editing SHALL NOT trigger decoration, restart its phase or accelerate its cadence.
Contour glyphs and positions SHALL remain fixed. Idle animation SHALL run at no
more than four frames per second; factual work indicators at no more than 12.5.
The startup reduced-motion override SHALL preserve static ornament and meaningful
state labels. Ambient decoration SHALL NOT invent actor activity.

#### Scenario: Settled idle interface
- **WHEN** the interface is idle and then the user edits or pastes input
- **THEN** only the draft and relevant editor feedback change; quiet ambient color
  continues on its existing clock without bursts, glyph flicker or new marks.

#### Scenario: Reduced motion
- **WHEN** KURU_REDUCED_MOTION=1 is set at launch
- **THEN** ambient ornament remains static while meaningful state and input
  updates remain available.

#### Scenario: Stable contours
- **WHEN** time advances through an ambient cycle without runtime events
- **THEN** contour characters, actor names and their positions remain identical,
  with only a gradual low-contrast color change over a 24-second period.
