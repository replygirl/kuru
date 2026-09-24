## MODIFIED Requirements

### Requirement: Explicit native publication outcomes

Publication SHALL provide new-only and checked-regular replacement semantics
using validated same-volume native operations with required flushing. Windows
checked replacement MUST retain the validated source, destination parent and
existing destination identities through one handle-relative native replacement;
an existing destination handle MUST remain bound to its replaced object while
later opens of the destination name resolve to the published source. Publication
MUST NOT copy/delete across volumes, overwrite an unchecked destination or
silently omit platform durability steps. Pre-publication rejection MUST preserve
old bytes. Possible post-move errors MUST expose uncertainty for caller identity
and receipt reconciliation rather than imply no mutation occurred. Domain
receipts and recovery policy SHALL remain with callers.

#### Scenario: Occupied or unsafe target
- **WHEN** a new-only target exists or a replacement target fails validation before the move
- **THEN** publication fails and the existing target remains byte-for-byte unchanged.

#### Scenario: Retained Windows replacement target
- **WHEN** checked Windows replacement retains an open validated destination handle through publication
- **THEN** one handle-relative native operation publishes the exact staged source, the retained destination handle still identifies and reads the replaced object, and later destination opens identify and read the published source.

#### Scenario: Native move has an uncertain result
- **WHEN** the move may have occurred before an error was returned
- **THEN** retained identity and outcome information allow the caller to determine actual state before retrying, without an automatic destructive rollback.
