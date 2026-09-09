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

The runtime SHALL offer explicit, periodic and session-end dreaming that can propose new peers and retire existing peers while preserving role coverage, configured size limits and recoverable prior topology.

#### Scenario: Invalid retirement
- **WHEN** dreaming proposes retiring the last active actor for a framework role
- **THEN** the proposal is rejected without deleting that actor's history.

#### Scenario: Restore prior topology
- **WHEN** the user reverses an applied dreaming change
- **THEN** the previous active membership is restored.

### Requirement: Resource budgets

The runtime MUST bound parallel model calls, peer rounds and tool calls, and permit cancellation.

#### Scenario: Cyclic peers
- **WHEN** peers repeatedly request each other
- **THEN** processing ends at the configured budget with an observable status.
