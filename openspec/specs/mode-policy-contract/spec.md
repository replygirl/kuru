# mode-policy-contract Specification

## Purpose
Define the four existing modes as validated, independently callable role, peering, flow, facing, visibility and memory policies while preserving their authored identities and equal-peer runtime behavior.

## Requirements

### Requirement: Six separable pure mode policies

The core SHALL expose independently callable roles, peering, flow, facing, visibility and memory policy operations in one validated profile for each of the four existing serialized modes. The operations SHALL accept bounded typed inputs and return decisions or plans; they MUST NOT hold provider, tool-host, memory-store or mutable runtime authority. The runtime SHALL retain execution, budgets, permission, private-memory isolation, cancellation and publication authority. P3 SHALL NOT replace the current runtime loops; P8 and P9 will consume all six policy axes.

#### Scenario: Four built-in profiles

- **WHEN** each existing `Mode` resolves its built-in profile
- **THEN** all six components exist, validate, and return decisions equivalent to the current built-in behavior without introducing another shipping mode or focus control.

#### Scenario: Independently replaceable decision

- **WHEN** a test supplies an alternative peering, flow, facing, visibility or memory component
- **THEN** its pure operation can produce a different decision without editing another component, while runtime validation can reject an unauthorized result.

### Requirement: Stable authored roles and equal authority

Built-in role policies SHALL reproduce the existing seed names, roles, stable IDs, instruction bytes, authored order and required-role coverage. Every generated peer instruction MUST retain the canonical equal-peer wrapper; no seed or facing selection SHALL gain supervisor, tool, memory or budget authority.

#### Scenario: Persisted seed identity

- **WHEN** each reference profile generates its seeds
- **THEN** their ordered IDs, names, roles and complete instructions match the pre-extraction golden values byte-for-byte.

#### Scenario: Invalid profile

- **WHEN** a profile duplicates a seed ID, omits authored order or required-role coverage, grants another identity's private history, or emits an invalid namespace
- **THEN** validation rejects it before activation without relaxing runtime invariants.

### Requirement: Current mode decisions remain expressible

The built-in policies SHALL express current all-active-or-targeted deliberation, direct active-peer routing, one-hop consultation, active-part relationship membership, target/focus/activation/continuity/authored/stable facing order, actor-private notes/history, bounded public transcript, exact namespace layout and candidate-only dreaming plan. The current runtime SHALL retain those behaviors until P8/P9 replace direct decisions with policy calls.

#### Scenario: Caller target and relationship focus

- **WHEN** an explicit target or live focused relationship is selected
- **THEN** the same identity wins with the existing reason, including a focused relationship with no draft and only explicitly supplied member contributions.

#### Scenario: Equal activation

- **WHEN** multiple successful drafts share maximum activation
- **THEN** the previous completed speaker wins if eligible, otherwise the first eligible authored seed wins, otherwise stable ascending ID order wins with the existing reason string.

#### Scenario: Isolated context and dream

- **WHEN** an actor or relationship is invoked, or dreaming requests consolidation
- **THEN** the policy plan names only its own private history and notes, the bounded public transcript and explicit inbound content, while dream writes remain on the runtime-owned candidate until validated promotion.

### Requirement: Pre-extraction behavior baseline

The repository SHALL retain deterministic baseline fixtures for all four modes before P8/P9 extraction. They SHALL compare representative provider requests, peer/relationship deliveries, speaker and reason, targeting, focus, activation, continuity, cold ties, budget outcomes, public/private context and dream fallback at their relevant runtime layer. Normalization SHALL exclude only incidental generated IDs and timing, not semantic differences.

#### Scenario: Scripted four-mode turn

- **WHEN** each built-in mode runs a scripted turn through the current runtime
- **THEN** assertions capture its seed order, request phases and tools, selected speaker/reason, contribution envelope and durable turn outcome for later equivalence comparison.

#### Scenario: Dream and recovery baseline

- **WHEN** a scripted dream proposal fails, succeeds, or is reversed
- **THEN** the baseline records preserved role coverage, candidate-only writes, stable namespaces and later-history-preserving undo without granting a policy mutation authority.

### Requirement: Explicit relationship proposal origin

The Peering relationship operation SHALL receive an explicit proposal origin:
either a direct User/API origin or a Peer origin carrying its checked live
identity and relationship-member context. The User/API path SHALL retain the
current direct relationship behavior without inventing a peer sender. A Peer
origin SHALL remain subject to current-participation validation: its proposer
is a proposed member, or every existing relationship member is proposed.
Canonical relationship identity, distinct active membership, and the two-to-
four member bounds remain runtime-validated.

#### Scenario: Direct user relationship proposal

- **WHEN** a direct user/API request proposes a valid relationship of active
  peers
- **THEN** Peering receives the User origin and may admit the same canonical
  relationship without requiring a synthetic peer identity.

#### Scenario: Nonparticipating peer proposal

- **WHEN** a live peer proposes a relationship without participating under the
  current membership rule
- **THEN** Peering rejects the proposal before relationship publication or
  durable state mutation.
