## MODIFIED Requirements

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
