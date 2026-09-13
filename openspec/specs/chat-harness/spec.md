# chat-harness Specification

## Purpose
Provide conventional terminal and scripted chat interfaces to the peer runtime,
with discoverable model, effort and framework selection, persistent sessions,
layered configuration and responsive cancellation.

## Requirements

### Requirement: Conventional interactive and scripted chat

The application SHALL provide a terminal chat interface with model, effort and mode controls, status, visible parts and relationships, input, cancellation, and a scripted run command with JSON output.

#### Scenario: Offline first run
- **WHEN** a user starts a demo conversation without provider credentials
- **THEN** a complete response is produced with inspectable participating peers.

### Requirement: Layered configuration and project instructions

The application SHALL merge user, ancestor project and explicit local
configuration in that order, reject unknown or invalid options, and load
ancestor AGENTS.md instructions with clear local precedence. It SHALL retain the
canonical workspace and a one-shot, side-effect-free configuration snapshot with
field-level provenance before activating automatic-ancestor authority. The
snapshot SHALL capture every CLI override, including `allow_write`,
`allow_shell`, and `no_dream`, before manifest derivation and review; no
post-snapshot mutation may add or alter authority. Explicit configuration and
CLI inputs authorize only their own effective leaves; saved preferences remain
limited to mode, model and effort and cannot add authority. Ordinary model
auto-resolution and runtime preference application MAY occur after the final
snapshot without rereading or changing authority configuration.

#### Scenario: Local override
- **WHEN** a project config changes the user default mode and explicit local config changes it again
- **THEN** the local mode is effective while unrelated inherited options remain.

#### Scenario: Ancestor authority pending approval
- **WHEN** an ancestor contributes an effective authority-bearing configuration
  leaf without a matching workspace approval
- **THEN** the parsed snapshot remains inspectable but runtime activation does
  not begin.

#### Scenario: Malformed automatic configuration
- **WHEN** an automatic or explicit configuration file has a read, TOML parse,
  type, or validation error containing fake secrets or terminal controls
- **THEN** Kuru reports only a bounded escaped source label, coarse error
  category, and parser-provided line and column when available, without raw
  parser text, excerpts, values, or control characters.

### Requirement: Durable session continuity

The application SHALL persist session state and histories outside tool roots and expose session inspection and resumption.

#### Scenario: Restart
- **WHEN** a process exits and a new process resumes the same session
- **THEN** its conversation and peer topology remain available.

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

### Requirement: Asynchronous terminal event scheduling

The interactive TUI SHALL consume native terminal input through one asynchronous
event stream and MUST NOT combine that stream with synchronous terminal polling
or reads. Terminal input, typed dispatch completion, bounded runtime activity and
the animation deadline SHALL wake the loop independently and receive bounded fair
service. Native resize and terminal input that become ready together MUST both be
consumed without requiring a later unrelated terminal event. Runtime activity MAY
decorate presentation but MUST NOT complete a turn,
replace a typed result, or indefinitely extend the activity drain performed before
typed completion.

Terminal input EOF, terminal read failure and rendering failure MUST terminate
through one cleanup boundary that aborts and awaits any TUI-owned dispatch job
and its nested runtime actor/provider work before native terminal state is
restored. This failure cleanup MUST NOT request exit dreaming. Activity-channel
closure SHALL only disable activity reception and MUST NOT complete work,
terminate the session or create a busy loop.

#### Scenario: Typed completion wakes an idle terminal

- **WHEN** a same-generation command or turn result becomes ready while no terminal input or animation tick is ready
- **THEN** the TUI applies that typed result without waiting for an unrelated polling interval, refreshes runtime presentation after the result, settles the operation and renders the completed state.

#### Scenario: Continuously ready sources remain fair

- **WHEN** terminal input or runtime activity remains continuously ready while a typed completion or elapsed animation deadline is also ready
- **THEN** the completion or deadline is serviced within one bounded scheduler rotation, and pre-completion activity receives are capped by a bounded snapshot of the queue length, including lag notifications.

#### Scenario: Resize and pasted input arrive together

- **WHEN** a native terminal resize and bracketed-paste input become ready in the same poll cycle
- **THEN** the sole asynchronous terminal stream reports both events without requiring a subsequent key, resize, or timer event.

#### Scenario: Stale and decorative events cannot complete a turn

- **WHEN** activity is lagged or closed, or a typed result arrives for an obsolete generation after cancellation
- **THEN** lag is represented only by the bounded activity notice, closure disables that source, and the stale result changes no current completion, runtime snapshot or operation state.

#### Scenario: Terminal input or rendering fails during work

- **WHEN** the sole terminal stream returns EOF or an error, or terminal drawing fails while a dispatch job is active
- **THEN** the TUI aborts and awaits that job, performs non-dream runtime shutdown until its nested actor/provider work is stopped, reports the original terminal failure with any cleanup failure as secondary context, and restores the native terminal modes it changed.

#### Scenario: Explicit cancellation remains ordered

- **WHEN** a user cancels an active turn and then submits another turn
- **THEN** the old job is aborted and awaited before its generation is fenced, queued activity is projected before the cancellation state is settled and refreshed, and no late old-generation result enters the next turn.
