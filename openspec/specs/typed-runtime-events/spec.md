# typed-runtime-events Specification

## Purpose
Define bounded, redacted typed runtime events with stable three-key wire projection, settled tool observations, and version-aware completed-turn replay.

## Requirements

### Requirement: Semantic event contract and compatible wire adapter

The runtime SHALL use a public semantic `Event` enum as the event type held by
the harness broadcast, trace, completed `TurnOutput`, and TUI callback paths.
Before an event enters any of those paths, one projection boundary SHALL redact
and bound its actor and every payload field. The public adapter SHALL serialize
every event as exactly `kind`, `actor`, and `detail`, preserving the existing
known kind names and detail forms; it SHALL add `tool-observation` for settled
tool observations. New known events SHALL NOT use the legacy escape variant.

#### Scenario: projected semantic event reaches all consumers
- **WHEN** the runtime emits a peer, relationship, state, speaker, or response event containing a projected payload
- **THEN** broadcast, trace, completed output, journal, and TUI receive the matching semantic variant and wire serialization contains only the compatible three keys

#### Scenario: unsafe event material is withheld
- **WHEN** projection of an event actor, kind, or payload fails or exceeds its bound
- **THEN** every public event path contains only the fixed withheld marker and no unprojected material

### Requirement: Settled tool observations

The runtime SHALL emit exactly one `ToolSettled` observation for each admitted
cognitive or external tool invocation, including successful, failed,
policy-denied, budget-failed, and cancelled calls. Its call ID, name, and
arguments SHALL be projected and bounded before measurement; its duration SHALL
span admission through settlement. Argument bytes SHALL equal the UTF-8 length
of serialized projected arguments. A result receipt SHALL contain only the
UTF-8 byte count and lowercase SHA-256 of the serialized projected result;
calls without a result SHALL record zero result bytes and no digest.

#### Scenario: receipt describes the actor-visible result
- **WHEN** a tool succeeds or returns a projected error receipt
- **THEN** its observation outcome, byte count, and digest can be recomputed from the bounded projected serialized value and reveal no result body

#### Scenario: denied and cancelled calls settle once
- **WHEN** a call is denied before execution, fails budget admission, or is cancelled after admission
- **THEN** the runtime emits one observation with the matching outcome or error classification, no result digest, and no duplicate settlement record

### Requirement: Version-aware completed-turn replay

New completed turn journals SHALL use format 2 and retain one output and one
wire-event array. Replay SHALL inspect the enclosing journal format before
normalizing events: v1 SHALL use the legacy reprojector, while v2 SHALL
strictly reconstruct known semantic variants. A malformed v2 known payload
SHALL normalize to a fixed withheld event; an unknown historical kind SHALL
remain a projected `Legacy` event. Completed replay and exact retry SHALL return
the stored final `TurnOutput` without provider or tool dispatch, a new settled
observation, or historical rewrite.

#### Scenario: v2 checkpoint survives reopen
- **WHEN** a completed v2 turn containing typed events is checkpointed, reopened, and replayed
- **THEN** its typed variants and compatible wire events match the stored projected output without dispatching new work

#### Scenario: historical journal remains safe
- **WHEN** a completed v1 journal or a v2 journal with malformed known event detail is replayed
- **THEN** v1 preserves its legacy projected behavior and malformed v2 detail becomes a withheld event without exposing raw stored content

### Requirement: Typed interactive event consumption

The TUI and CLI event consumers SHALL consume projected semantic events without
reparsing generic event detail to recover peer routing, relationship, state,
speaker, or response meaning. The TUI SHALL continue to show useful projected
activity while never rendering a peer message body, and final response text
SHALL remain authoritative in `TurnOutput` rather than event detail.

#### Scenario: TUI renders useful typed activity
- **WHEN** the view receives projected peer, relationship, state, speaker, and response events
- **THEN** it derives recipient and visible state from typed fields, omits peer message content, and retains the existing final-answer behavior

### Requirement: Concurrent tool activity retains exact settlement

Projected tool activity SHALL show every actually started call until that call settles, including same-name calls and out-of-order settlements. The runtime SHALL emit exactly one bounded settled observation per admitted call with its original provider call ID and result receipt. Its compatible three-key wire adapter SHALL retain the existing tool-start and tool-observation forms and SHALL NOT expose raw arguments or result bodies to the activity label.

#### Scenario: Two active calls settle in reverse order

- **WHEN** two same-actor tool calls start and the later call settles first
- **THEN** the view still shows the earlier call active, each settlement clears only its own activity, and both projected observations retain their distinct call IDs

### Requirement: Hook outcomes use bounded semantic projection

The runtime SHALL project hook start and terminal outcomes through typed bounded data that identifies the event kind, configured hook identity, related turn or call identity, and outcome classification without raw command paths, arguments, stdin, stdout, stderr, rewritten payloads, settled result bodies, or annotation bodies. Valid annotations SHALL remain separately typed hook output subject to their receiving actor's visibility and context-fit policy. Post-turn hooks SHALL run only after the answer is settled and SHALL report their bounded outcomes separately to the initiating caller without changing the completed answer or journal. Completed replay MUST return the stored settled output and MUST NOT execute a hook again; it need not reconstruct later post-turn observations as part of that immutable output.

#### Scenario: Hook failure is safe to render

- **WHEN** a hook fails with hostile stdout, stderr, path text, and terminal controls
- **THEN** the live trace, CLI, and TUI receive only the bounded projected failure and no hostile material; any outcome recorded before settlement in the journal or completed output is bounded too

#### Scenario: Exact retry replays hook outcome

- **WHEN** a completed turn is retried by its exact identity after post-turn observation
- **THEN** Kuru returns its stored settled output without rerunning the hook, provider, or tool; the separate post-turn observation is not presented as a new durable turn outcome
