# peer-cognition Specification

## Purpose
Define equal persistent framework actors, bounded direct peer communication,
isolated memories and temporary speaking relationships. Preserve role coverage,
memory ownership and reversibility as dreaming changes the pool's membership.

## Requirements

### Requirement: Equal persistent framework actors

The runtime SHALL provide IFS, polyvagal, Freudian and Jungian modes as pools of peers with stable identities and isolated durable histories, without a supervisory LLM owning the pool.

#### Scenario: Peer continuation
- **WHEN** one actor addresses a different active actor
- **THEN** the runtime delivers the message directly within the configured execution budget and records its source and destination.

#### Scenario: Private history
- **WHEN** an actor is invoked after another actor stored private content
- **THEN** the unrelated actor's private content is absent from the invocation context.

### Requirement: Persistent temporary relationships

The runtime SHALL validate protection, polarization and alliance relationships of two to four distinct active peers, allow a relationship to speak to the user, and isolate its durable history from nonmembers.

#### Scenario: Relationship reactivation
- **WHEN** the same members reactivate the same relationship kind after another identity spoke
- **THEN** its canonical identity and previous relationship memory are retained.

### Requirement: Bounded reversible dreaming

The runtime SHALL offer explicit, periodic and session-end dreaming that can
propose new peers and retire existing peers while preserving role coverage,
configured size limits and recoverable prior topology. A newly accepted peer
addition SHALL persist a canonical equal-peer instruction consisting exactly of
`PEER_INSTRUCTION + "\n\nYour tendency: "` and the proposal's nonblank authored
tendency. The core instruction constructor SHALL check the incoming
instruction's existing 8,192-byte limit before canonicalization. Canonicalization
SHALL remove only repeated exact copies of that complete leading canonical
prefix and SHALL preserve the remaining tendency bytes; blank or prefix-only
remainders SHALL be rejected. The bounded rule applies again to every new
proposal, so an already-wrapped persisted instruction larger than 8,192 bytes
is not thereby valid as new input. Builtin peer identities and instructions,
existing persisted peers, reload, and historical branches SHALL not be
rewritten by this construction.

The complete dream SHALL use an isolated candidate memory view and promote only
from its recorded live base. Cancellation and storage failure MUST NOT expose
partial active memories. Individual model/proposal errors SHALL remain report
rejections. Undo SHALL add a compensating revision, retaining conversations and
preferences written afterward.

#### Scenario: Canonical accepted addition
- **WHEN** a valid dream `Add` proposal supplies an unwrapped nonblank tendency
- **THEN** its promoted new peer has exactly one canonical equal-peer prefix followed by that unchanged tendency.

#### Scenario: Repeated leading canonical prefix
- **WHEN** a valid dream `Add` proposal begins with one or more exact complete canonical prefixes followed by a nonblank tendency
- **THEN** the promoted new peer has one canonical prefix and the same remaining tendency bytes, without altering similar text elsewhere in that tendency.

#### Scenario: Bound before normalization
- **WHEN** an incoming dream `Add` instruction exceeds 8,192 bytes, including an otherwise canonical wrapped result
- **THEN** the proposal is rejected before normalization and other independently valid proposals retain their accepted result.

#### Scenario: Prefix-only addition
- **WHEN** a dream `Add` instruction is blank or contains only repeated complete canonical prefixes and whitespace
- **THEN** the proposal is rejected and no new active peer is persisted.

#### Scenario: Restore prior topology
- **WHEN** the user reverses an applied dreaming change
- **THEN** the previous active membership is restored.

#### Scenario: Invalid retirement
- **WHEN** dreaming proposes retiring the last active actor for a framework role
- **THEN** the proposal is rejected without deleting that actor's history.

#### Scenario: Interrupted dream
- **WHEN** a dream is cancelled after a peer stores candidate notes and before promotion is accepted
- **THEN** those notes and candidate topology remain absent from live memory.

#### Scenario: Interrupted promotion reply
- **WHEN** cancellation interrupts the reply to an accepted promotion
- **THEN** the complete candidate may become active atomically and its durable outcome is reconciled before further runtime work.

#### Scenario: Later conversation survives undo
- **WHEN** the user chats and changes preferences after a dream, then undoes it
- **THEN** previous active membership returns while later messages and preferences remain durable after reopening.

### Requirement: Resource budgets

The runtime MUST bound parallel model calls, peer rounds and tool calls, and permit cancellation.

#### Scenario: Cyclic peers
- **WHEN** peers repeatedly request each other
- **THEN** processing ends at the configured budget with an observable status.

### Requirement: Stable speaker selection

The runtime SHALL preserve existing caller-target and active-focus precedence.
For automatic selection it SHALL choose only among successful deliberating peers,
prefer the greatest reported activation, retain the previous completed speaker
when that peer shares the greatest activation, and otherwise choose the first
identity in stable ascending order. Turn count MUST NOT change this tie-break.
The last completed speaker SHALL persist with session state and old sessions
without that value SHALL remain readable. Selection reasons SHALL be visible in
the event trace without changing existing speaker-event meaning or peer authority.

#### Scenario: Equal activations across turns and resume
- **WHEN** the same eligible peers share the highest activation across completed turns or after resuming the session
- **THEN** the previous completed speaker remains selected independently of turn count.

#### Scenario: A different peer has greater activation
- **WHEN** an eligible peer has strictly greater activation than the previous speaker
- **THEN** the higher-activation peer is selected and the reason identifies activation as decisive.

#### Scenario: Missing or ineligible prior speaker
- **WHEN** there is no previous completed speaker or that identity is not among the eligible maximum-activation peers
- **THEN** the stable identity tie-break selects an eligible maximum without creating or reviving an actor.

#### Scenario: Turn fails before completion publication
- **WHEN** speaking fails or is cancelled before publishing completed session state
- **THEN** it does not replace the session's last completed speaker.

#### Scenario: Existing target and focus behavior
- **WHEN** an existing caller target or valid active focus selects the identity
- **THEN** that precedence remains effective and its reason is observable without requiring a new command or setting.
