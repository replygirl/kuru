# chat-harness Specification

## Purpose
Provide conventional terminal and scripted chat interfaces to the peer runtime,
with discoverable model, effort and framework selection, persistent sessions,
layered configuration and responsive cancellation.

## Requirements

### Requirement: Conventional interactive and scripted chat

The application SHALL provide a terminal chat interface with model, effort and mode controls, status, visible parts and relationships, input, cancellation, and a scripted run command with JSON output. After actual writable project memory is available, a pending first-run memory notice SHALL be written and flushed to stderr before headless runtime work, or shown as a system transcript entry only after the first successful completed TUI draw and before input admission. The notice MUST use the escaped actual managed directory and list the notes, export, selected-forget, and purge-help controls. It MUST remain outside model prompts, peer histories, `TurnOutput`, and machine stdout. Help, inspection, configuration, authentication, models, tools, sessions, and missing-store paths MUST NOT open memory merely to show or record it.

#### Scenario: Offline first run
- **WHEN** a user starts a demo conversation without provider credentials
- **THEN** a complete response is produced with inspectable participating peers.

#### Scenario: Pending interactive notice
- **WHEN** a first interactive writable project reaches its completed initial frame
- **THEN** the frame shows the informational memory controls before input and records the version only after that draw succeeds.

#### Scenario: Headless JSON notice
- **WHEN** a pending writable project runs a headless runtime command with JSON output
- **THEN** stdout remains parseable JSON while the notice is flushed on stderr before work and is not repeated after reopening.

#### Scenario: Provider-free undo notice
- **WHEN** a pending writable project runs `undo-dream`
- **THEN** it follows the same stderr-before-work and durable-version boundary without starting a provider.

### Requirement: Layered configuration and project instructions

The application SHALL merge user, ancestor project and explicit local
configuration in that order and reject unknown or invalid options. It SHALL
capture every automatically discovered ancestor `AGENTS.md`, including the
project root, in outermost-to-most-local order and load only the captured bytes
into prompts after the applicable workspace approval. It SHALL retain the
canonical workspace and a one-shot, side-effect-free configuration and
instruction snapshot with field-level provenance before activating
automatic-ancestor authority. The snapshot SHALL capture every CLI override,
including `allow_write`, `allow_shell`, and `no_dream`, before manifest
derivation and review; no post-snapshot mutation may add or alter authority or
prompt instructions. Explicit configuration and CLI inputs authorize only
their own effective leaves; saved preferences remain limited to mode, model and
effort and cannot add authority. Ordinary model auto-resolution and runtime
preference application MAY occur after the final snapshot without rereading or
changing authority configuration or instruction sources.

#### Scenario: Local override
- **WHEN** a project config changes the user default mode and explicit local config changes it again
- **THEN** the local mode is effective while unrelated inherited options remain.

#### Scenario: Ancestor authority pending approval
- **WHEN** an ancestor contributes an effective authority-bearing configuration
  leaf or automatic instruction source without a matching workspace approval
- **THEN** the parsed snapshot remains inspectable but runtime activation and
  instruction injection do not begin.

#### Scenario: Exact reviewed instruction bytes
- **WHEN** an automatic instruction file changes after snapshot creation and
  before runtime construction
- **THEN** that invocation injects the ordered bytes owned by its approved
  snapshot and does not reopen the changed pathname.

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
decorate presentation but MUST NOT complete a turn, replace a typed result, or
indefinitely extend the activity drain performed before typed completion.

Normal explicit cancellation MUST signal the active operation, await its typed
settled result, apply an authoritative completed answer when that answer won, and
only then fence the old generation. Terminal input EOF, terminal read failure and
rendering failure MUST terminate through one cleanup boundary that signals and
awaits a TUI-owned dispatch job when its cancellation token is available, or
aborts and awaits the job when no such token remains. That boundary MUST stop
nested runtime actor/provider work before native terminal state is restored and
MUST NOT request exit dreaming. Activity-channel closure SHALL only disable
activity reception and MUST NOT complete work, terminate the session or create a
busy loop.

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

- **WHEN** activity is lagged or closed, or a typed result arrives for an obsolete generation after settled cancellation
- **THEN** lag is represented only by the bounded activity notice, closure disables that source, and the stale result changes no current completion, runtime snapshot or operation state.

#### Scenario: Terminal input or rendering fails during work

- **WHEN** the sole terminal stream returns EOF or an error, or terminal drawing fails while a dispatch job is active
- **THEN** the TUI signals and awaits that job when its cancellation token is available, otherwise aborts and awaits it, performs non-dream runtime shutdown until nested actor/provider work is stopped, reports the original terminal failure with any cleanup failure as secondary context, and restores the native terminal modes it changed.

#### Scenario: Explicit cancellation remains ordered

- **WHEN** a user cancels an active turn and then submits another turn
- **THEN** the TUI signals cancellation, awaits and applies the typed interruption or winning answer, projects queued activity before settling and refreshing the operation, fences the old generation, and admits the next turn without a late old-generation result.

### Requirement: Durable idempotent turn lifecycle

The harness SHALL offer a controlled turn entrypoint with a bounded caller-supplied turn ID and an explicit cancellation token while retaining convenience entrypoints that create fresh IDs and tokens. Turn identity SHALL be scoped to the canonical project and session. Within that scope the harness MUST compare the exact prompt and raw target before reusing an ID. A completed matching ID MUST return its stored authoritative `TurnOutput` without another provider, tool, cognitive, or A2A dispatch; mismatched reuse and a possibly dispatched incomplete turn MUST fail explicitly rather than replay. The same ID MAY identify an independent turn in another session.

#### Scenario: Completed retry
- **WHEN** a caller repeats a completed turn ID with the same session, prompt, and target
- **THEN** the harness returns the same authoritative output without another provider or tool invocation or another transcript row.

#### Scenario: Mismatched or uncertain retry
- **WHEN** a caller reuses an ID for a different request or for an incomplete turn that may have dispatched external work
- **THEN** the harness reports the mismatch or uncertain status and performs no provider, tool, cognitive, or A2A dispatch.

#### Scenario: Session-scoped reuse
- **WHEN** two sessions in the same project use the same turn ID for independent requests
- **THEN** each session journals and completes its own turn without a global ID conflict.

### Requirement: Explicit cancellation and answer boundary

The runtime SHALL propagate one cancellation signal through turn admission, actor queue and semaphore waits, provider asks, built-in and MCP tools, cognitive calls, A2A, and dreaming. Once a memory mutation is accepted, it MUST finish or reconcile before another mutation; owned subprocess cleanup MUST remain independent of the cancelled caller. A turn SHALL become complete at its durable ended checkpoint before response publication and periodic dreaming. Cancellation after that boundary MUST preserve and return the completed answer. `TurnOutput` SHALL retain its existing JSON shape, with its event trace frozen at the answer boundary; later dream events are maintenance activity.

#### Scenario: Cancellation loses or wins the completion race
- **WHEN** interactive cancellation settles before the ended checkpoint
- **THEN** the TUI reports an interrupted turn whose user prompt remains in conversation history and permits the next operation.
- **WHEN** the ended checkpoint settles first
- **THEN** the TUI displays the authoritative answer even if cancellation or periodic dream shutdown follows.

#### Scenario: Post-answer dream stops
- **WHEN** periodic dreaming fails or is cancelled after the ended checkpoint
- **THEN** the completed `TurnOutput` remains retrievable and dream diagnostics appear only as later maintenance activity.

### Requirement: Bounded shutdown dreaming

Shutdown SHALL give optional dreaming one finite aggregate deadline and SHALL attempt normal actor and tool-host cleanup after dream success, failure, cancellation, or timeout. A deadline or channel closure MUST NOT be treated as proof that an owned subprocess was cleaned.

#### Scenario: Shutdown dream exceeds its deadline
- **WHEN** a provider stalls during shutdown dreaming
- **THEN** Kuru cancels that dream, attempts actor and tool-host cleanup, and returns an honest bounded failure if cleanup or dreaming remains unconfirmed.

### Requirement: Dream candidate resolution

Dream failure or cancellation SHALL settle accepted candidate writes before it
explicitly abandons the candidate, and abandonment cleanup SHALL continue in an
owned worker if its caller stops waiting. Process loss or abnormal termination
without a completed explicit transition MUST leave the ordinary candidate
recoverable. A confirmed fast-forward SHALL remain the authoritative promoted
outcome even if later candidate-ref cleanup is incomplete, so cleanup failure
MUST NOT suppress live runtime publication.

#### Scenario: Settled dream is abandoned

- **WHEN** a dream fails or observes cancellation after creating its candidate
- **THEN** accepted writes settle before explicit abandonment, exact resolved-ref cleanup is attempted without replay, and an unconfirmed result remains recoverable.

#### Scenario: Promotion wins cleanup failure

- **WHEN** the candidate fast-forward is confirmed but candidate-ref cleanup fails or loses its reply
- **THEN** the runtime publishes the promoted topology and memory exactly once while retained cleanup state remains safe to retry.

#### Scenario: Process stops without explicit resolution

- **WHEN** a process stops while an ordinary candidate has no durable promotion or abandonment transition
- **THEN** the candidate branch and its private history remain intact and startup does not delete or promote it.

### Requirement: Reachable exact retry and durable interruption presentation

The scripted `run` command SHALL accept an optional bounded caller turn ID, and
the TUI SHALL offer `/retry` only for its single last durably retained local
submission. A new local submission MUST atomically retain its exact ID, prompt
and raw target with the started journal and sole user transcript row; retry MUST
reuse that tuple without replacing it. A completed retry MUST perform no new
provider/tool work and MUST NOT duplicate the displayed or durable user/answer
pair. A possibly dispatched incomplete turn MUST refuse retry without side
effects. A2A message-ID idempotency SHALL remain separate and MUST NOT replace
the local TUI reference.

When an admitted turn settles without a completed output, one fixed nonsecret
interruption marker MUST be committed through the journal/transcript path and
remain visible after later turns and resume. The marker MUST NOT claim that
external work was absent or rolled back and MUST be excluded from provider
conversation context. Repeated settlement MUST NOT duplicate it. If the ended
answer checkpoint wins a race, the authoritative answer SHALL be shown and no
interruption marker added.

#### Scenario: Completed last-turn retry
- **WHEN** `/retry` or `run --turn-id` exactly repeats a completed local submission in its original session
- **THEN** the stored output is reused with zero provider/tool calls and no duplicate transcript or TUI pair.

#### Scenario: Unsafe last-turn retry
- **WHEN** the retained turn may have dispatched external work
- **THEN** `/retry` refuses without calls and explains that a new submission is new work.

#### Scenario: Safe pre-dispatch retry
- **WHEN** a retained turn was interrupted before possible dispatch
- **THEN** `/retry` may complete that same logical turn without another user row, retaining the earlier interruption marker.

#### Scenario: Persistent interruption
- **WHEN** an interruption settles and later work completes or the session is resumed
- **THEN** exactly one Kuru interruption marker remains in the transcript without entering provider context.

#### Scenario: Completion wins cancellation
- **WHEN** the ended checkpoint commits before cancellation is reconciled
- **THEN** the committed answer is returned and displayed without a false interruption marker.

### Requirement: Ephemeral selected-speaker preview

The runtime SHALL expose bounded replaceable text and visible-summary snapshots
only for the selected outward-facing speaking invocation, including a selected
relationship speaker. Deliberation, consultations, dreams, raw reasoning and tool
fragments MUST NOT appear in that preview. Each speaking request round SHALL
replace prior provisional content. The TUI SHALL show a partial-line tail and a
separate visible-summary or truthful activity region without adding either to the
saved transcript. Coalescing preview updates MUST preserve responsive input,
cancellation and terminal-result handling.

#### Scenario: Delayed outward reply

- **WHEN** the selected speaker streams text and an explicitly visible summary
  while other identities perform private requests
- **THEN** a real terminal displays provisional text and summary before terminal
  completion, without any private peer, consultation or dream text.

#### Scenario: Bounded burst and resizing

- **WHEN** stream updates exceed paint cadence or the terminal resizes between
  120, 80 and 40 columns
- **THEN** the preview retains a bounded usable tail, identifies truncation, and
  the composer, cancellation and eventual completed response remain usable.

### Requirement: Preview settlement and stale-operation fencing

Preview identity SHALL include admitted turn, speaking request round and
monotonic sequence; the TUI SHALL additionally fence updates by its active
operation generation. Cancellation, quit, a new operation and final completion
SHALL clear provisional state and reject late snapshots. Only the authoritative
completed turn SHALL enter the transcript and existing final-only CLI output.
An interrupted preview MUST NOT be journaled, replayed on resume or returned as
a completed exact retry. Existing completed-checkpoint precedence over a racing
cancellation and possible-dispatch retry refusal MUST be preserved.

#### Scenario: Tool-loop preview replacement

- **WHEN** an outward request proposes tools and another speaking round follows
- **THEN** the next round replaces the earlier draft, and final completion appears
  once with no duplicated preview text in stored history or turn output.

#### Scenario: Cancel and late update

- **WHEN** a provisional reply is cancelled and an old snapshot arrives after a
  new operation starts
- **THEN** it cannot alter the new preview or transcript, and resuming the session
  shows only existing durable conversation and interruption state.

#### Scenario: Exact completed retry

- **WHEN** a completed turn is retried or read through scripted JSON output
- **THEN** its authoritative saved output is returned without another inference
  dispatch or a replayed preview.

### Requirement: Foreground tool approval and permission review

The attached interactive terminal SHALL offer bounded, redacted inline once/session/always/deny choices for an ask decision, showing the exact scope that a session or always grant would cover. The default file grant scope SHALL be the validated exact project-relative target; shell, MCP and outbound A2A SHALL show their whole-tool identity. The terminal SHALL expose current permission state through a dock chip and `/permissions` inspection with session/always revocation. A prompt MUST NOT hold a memory transaction or UI lock, and cancellation, quit, a closed approval channel or a newer operation MUST fence a late decision. Headless/scripted output SHALL remain final-only and shall not present an interactive approval prompt.

#### Scenario: Exact-file approval and revocation
- **WHEN** a user approves an asked file operation for this session and later revokes its displayed exact-file scope through `/permissions`
- **THEN** the first checked operation executes, another path receives a new decision, and the revoked scope authorizes no later call.

#### Scenario: Persistent whole-tool approval
- **WHEN** a user approves a displayed MCP, shell or outbound A2A whole-tool scope as always and restarts under the same checked workspace/authority context
- **THEN** the checked grant remains visible and usable until revoked, without rendering raw arguments or secrets.

#### Scenario: Cancellation or denied prompt
- **WHEN** the user denies, cancels or closes an inline request before a decision is applied
- **THEN** no effect is dispatched, the composer remains usable, and a late approval cannot authorize a newer operation.

### Requirement: Inspectable session cost and request fit

The TUI SHALL provide `/cost` and an estimated context indicator for the relevant active request, alongside existing model, effort, mode and permission state. It SHALL distinguish actual reported token counts from incomplete components, unknown prices, assumed model windows and subscription API-equivalent estimates; it MUST NOT present an estimate as a bill, remaining subscription quota or known zero. When optional history is omitted for fit, the user SHALL see its actual count without a false claim that persisted history was deleted. Headless output SHALL keep its existing machine format and may expose these facts only through an explicit compatible structured field or command output.

#### Scenario: Partial ledger and assumed window
- **WHEN** `/cost` inspects a resumed pre-ledger session using a model without a verified window or complete price
- **THEN** known usage appears with incomplete history and price labels, and the status identifies the assumed context bound.

#### Scenario: Narrow terminal with concurrent status
- **WHEN** the TUI shows context, cost and permission status in a narrow pane while a request is active
- **THEN** the current selection and operation controls remain usable and no provisional preview or private continuation becomes durable cost text.
