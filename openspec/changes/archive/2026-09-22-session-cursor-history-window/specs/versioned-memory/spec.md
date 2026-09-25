## ADDED Requirements

### Requirement: Bounded session cursor history projection

Memory SHALL expose a read-only newest-history suffix for one exact namespace and session strictly after an exclusive durable sequence. One checked result MUST carry the pinned live or candidate view, captured revision, requested cursor, exact count of all eligible rows and the newest complete sequenced rows that fit the requested limit and shared session-source byte bound. The result MUST preserve ascending durable order, MUST exclude rows at or before the cursor and rows from every other namespace, session or unattributed legacy history, and MUST behave identically through local and managed views without paging, export access or mutation authority.

#### Scenario: More eligible rows than one source page

- **WHEN** one session has more than 1,024 raw rows after its summary cursor and sibling namespaces and sessions have interleaved global sequences
- **THEN** a bounded request returns only that session's newest requested sequenced suffix in ascending order and reports the exact count of all eligible rows.

#### Scenario: Exact cursor has no later rows

- **WHEN** the cursor equals the newest eligible sequence or a zero-limit count is requested
- **THEN** the projection returns no rows, retains the exact view, revision and cursor, and reports the exact eligible-row count without treating omitted rows as absent.

#### Scenario: Candidate history stays isolated

- **WHEN** a candidate appends session rows after its captured base while live memory advances independently
- **THEN** the candidate projection returns only its pinned candidate suffix and revision while the live projection excludes unpromoted candidate rows.
