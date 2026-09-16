## MODIFIED Requirements

### Requirement: Explicit projections without new product surfaces

Text answer, CLI and TUI projections SHALL derive only from intended text blocks,
preserve current text joining and final-answer authority, and retain the existing
final TurnOutput JSON fields. Typed representations MUST NOT cause raw or
encrypted reasoning to enter persistent history, public transcript, events or
export. Provider-supplied visible reasoning summaries MAY appear transiently for
the selected speaking request, but MUST NOT be newly captured in durable history
or replayed as a persisted-reasoning product. Image and cache types remain
foundations without automatic media fetching or new media input. Unsupported
request content MUST fail before provider dispatch rather than be silently
omitted. Inspection/export SHALL identify structured blocks explicitly rather
than present their encoding as ordinary prose.

#### Scenario: Unsupported request media

- **WHEN** an unsupported image or other non-dispatchable block reaches the current text/provider boundary
- **THEN** the request fails explicitly before HTTP dispatch, without dropping the block or fetching media.

#### Scenario: Text answer and retry

- **WHEN** a text turn completes and its completed turn ID is retried
- **THEN** the same final answer and existing JSON fields are returned without another dispatch or transcript insert.

#### Scenario: Transient visible summary

- **WHEN** the selected speaking request supplies a visible summary
- **THEN** it may appear in the live preview but new durable assistant history,
  semantic events and final turn output do not acquire that summary.
