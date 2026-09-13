## ADDED Requirements

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

## MODIFIED Requirements

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
