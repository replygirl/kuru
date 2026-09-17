## ADDED Requirements

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
