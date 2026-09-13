## ADDED Requirements

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
before native terminal state is restored. Activity-channel closure SHALL only
disable activity reception and MUST NOT complete work, terminate the session or
create a busy loop.

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
- **THEN** the TUI aborts and awaits that job, reports the original terminal failure, and restores the native terminal modes it changed.

#### Scenario: Explicit cancellation remains ordered

- **WHEN** a user cancels an active turn and then submits another turn
- **THEN** the old job is aborted and awaited before its generation is fenced, queued activity is projected before the cancellation state is settled and refreshed, and no late old-generation result enters the next turn.
