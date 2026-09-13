## MODIFIED Requirements

### Requirement: Memory inspection

Kuru SHALL expose project memory status and revision history, and SHALL expose
provider-free selected-mode notes for one resolved part or relationship identity.
Each current note row MUST retain its stored signed sequence and role in the
projection. An explicit selected-note forget command MUST resolve the same
identity and exact `/notes` namespace, remove only the requested current row by
that namespace and sequence through an atomic versioned mutation, and preserve
unrelated notes, conversations, identities, and prior Dolt revisions. It MUST
refuse an absent store before legacy import or state creation and MUST disclose
that the operation does not erase historical revisions or other related text.
Kuru SHALL document managed runtime configuration, migration, offline use and
stopped-store backup/recovery.

#### Scenario: Selected current note is forgotten
- **WHEN** a user requests an existing selected identity's stored note sequence
- **THEN** the active `/notes` namespace omits only that row after one committed mutation and the result discloses retained history

#### Scenario: Unknown or non-note sequence is rejected
- **WHEN** a user requests a missing sequence or an identity whose current notes namespace does not contain it
- **THEN** Kuru reports the exact request failure without deleting a transcript, another identity's note, or any other current note

#### Scenario: Revision inspection
- **WHEN** the user requests memory history after a committed update
- **THEN** the command reports the durable revision without exposing credentials or unrelated projects.
