## ADDED Requirements

### Requirement: Discriminated typed message persistence

Memory SHALL distinguish legacy text rows from typed block payloads using an
explicit durable content-format discriminator introduced by an ordered migration.
Legacy content bytes, sequence, namespace, role, revisions and retained candidates
MUST remain intact. Readers MUST select the codec and available columns according
to the viewed schema, including supported historical and candidate schemas. An
unknown format or malformed typed payload MUST fail explicitly without a text
fallback. Typed writes to an old-schema view MUST be refused, while supported
legacy text operations remain compatible. Typed message checkpoint publication
and journal/state publication MUST remain one reconciled operation.

#### Scenario: Upgrade with JSON-looking legacy content

- **WHEN** a released old-schema store containing JSON-looking message strings upgrades and reopens
- **THEN** each old row remains exact text, new typed rows round trip as blocks, and retained old-schema revisions/candidates remain readable.

#### Scenario: Interrupted publication

- **WHEN** typed checkpoint publication has an uncertain reply or caller cancellation
- **THEN** existing receipt reconciliation determines its outcome without duplicate messages or a split message/state commit.

#### Scenario: Unsupported stored format

- **WHEN** a reader encounters an unknown content format or malformed typed payload
- **THEN** it reports a bounded storage error rather than partial or silently flattened history.

### Requirement: Typed export and legacy import compatibility

Revision-pinned exports SHALL carry the message content-format discriminator and
exact stored content, with an explicit export-format version that prevents typed
JSON being mistaken for legacy text. Markdown SHALL describe structured blocks
truthfully. Legacy SQLite import MUST retain source bytes and import messages as
legacy text; this change MUST NOT infer typed semantics from string contents or
change explicit forgetting, purge or no-expiry guarantees.

#### Scenario: Mixed-format export

- **WHEN** a committed store containing legacy text and typed messages is exported as JSON or Markdown
- **THEN** content, format, provenance and counts are preserved, and structured content is distinguishable from literal legacy prose.

#### Scenario: Read-only legacy import

- **WHEN** an existing supported SQLite fixture is imported into the new schema
- **THEN** source bytes remain unchanged and imported strings retain their original text interpretation.
