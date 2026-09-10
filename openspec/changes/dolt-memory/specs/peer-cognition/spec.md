## MODIFIED Requirements

### Requirement: Bounded reversible dreaming

The runtime SHALL offer explicit, periodic and session-end dreaming that can
propose new peers and retire existing peers while preserving role coverage,
configured size limits and recoverable prior topology. The complete dream SHALL
use an isolated candidate memory view and promote only from its recorded live
base. Cancellation and storage failure MUST NOT expose partial active memories.
Individual model/proposal errors SHALL remain report rejections. Undo SHALL add a
compensating revision, retaining conversations and preferences written afterward.

#### Scenario: Invalid retirement
- **WHEN** dreaming proposes retiring the last active actor for a framework role
- **THEN** the proposal is rejected without deleting that actor's history.

#### Scenario: Restore prior topology
- **WHEN** the user reverses an applied dreaming change
- **THEN** the previous active membership is restored.

#### Scenario: Interrupted dream
- **WHEN** a dream is cancelled after a peer stores candidate notes and before promotion is accepted
- **THEN** those notes and candidate topology remain absent from live memory.

#### Scenario: Interrupted promotion reply
- **WHEN** cancellation interrupts the reply to an accepted promotion
- **THEN** the complete candidate may become active atomically and its durable outcome is reconciled before further runtime work.

#### Scenario: Later conversation survives undo
- **WHEN** the user chats and changes preferences after a dream, then undoes it
- **THEN** previous active membership returns while later messages and preferences remain durable after reopening.
