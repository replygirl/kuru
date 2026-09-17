## MODIFIED Requirements

### Requirement: Current mode decisions remain expressible

The built-in policies SHALL express current all-active-or-targeted
deliberation, direct active-peer routing, one-hop consultation, active-part
relationship membership, target/focus/activation/continuity/authored/stable
facing order, actor-private notes/history, bounded public transcript, exact
namespace layout and candidate-only dreaming plan. P8 SHALL consume Roles,
Peering, Flow and Facing at their existing runtime effect boundaries; P9 SHALL
consume Visibility and Memory there. Runtime validation SHALL retain current
input, required receipts and continuation material, equal peers, budgets,
permission, cancellation, persistence-before-publication and candidate-only
dream authority.

#### Scenario: Caller target and relationship focus

- **WHEN** an explicit target or live focused relationship is selected
- **THEN** the same identity wins with the existing reason, including a focused
  relationship with no draft and only explicitly supplied member
  contributions.

#### Scenario: Equal activation

- **WHEN** multiple successful drafts share maximum activation
- **THEN** the previous completed speaker wins if eligible, otherwise the first
  eligible authored seed wins, otherwise stable ascending ID order wins with
  the existing reason string.

#### Scenario: Isolated context and dream

- **WHEN** an actor or relationship is invoked, or dreaming requests
  consolidation
- **THEN** the runtime validates and consumes only its own private history and
  notes, the bounded public transcript and explicit inbound content, while
  dream writes remain on the runtime-owned candidate until validated promotion.

## ADDED Requirements

### Requirement: Mandatory explicit input and bounded consolidation validation

Visibility-source validation SHALL require `ExplicitInput` exactly once in an
actor request plan and SHALL continue to reject another identity's private
history or notes. Consolidation-plan validation SHALL accept a unique ordered
subset of the actual live active parts, require nonempty prompt and descriptive
phase text, and bound each participant to at most two proposals. It SHALL run
against the actual topology on every dream; it MUST NOT infer validity from a
profile's seed list. Built-in plans SHALL retain their exact participants,
prompt, phase and two-proposal ceiling.

#### Scenario: Alternate live subset

- **WHEN** an alternate memory policy returns a unique ordered proper subset of
  the actual active parts
- **THEN** validation accepts that plan without changing the built-in plan.

#### Scenario: Missing input or invalid participant

- **WHEN** a source plan lacks `ExplicitInput`, or a consolidation plan repeats
  a participant, names an inactive identity, has blank text or exceeds the
  proposal ceiling
- **THEN** validation rejects it before the runtime reads context or starts a
  dream request.
