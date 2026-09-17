## ADDED Requirements

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
