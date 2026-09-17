# content-block-messages Specification

## Purpose

Provide one ordered, typed representation of conversation content across
providers, runtime and durable memory, preserving tool identities, explicit
usage observations and supported legacy history without confusing structured
records with ordinary text.

## Requirements

### Requirement: Canonical ordered message content

Messages and completions SHALL have a single canonical ordered content-block
representation for text, tool use, tool result, reasoning summaries, images and
cache boundaries. Tool IDs, JSON arguments/results and content order MUST survive
serialization. Roles and stop-reason strings MUST preserve unfamiliar values.
Completion usage MUST distinguish absent observations from observed zero and
represent input/output totals plus cached-input and reasoning-output components.
Text and tool-call convenience accessors MUST derive from those canonical blocks
rather than maintain independent mutable copies. Unsupported block tags MUST
fail explicitly instead of disappearing or falling back to text.

#### Scenario: Mixed content round trip

- **WHEN** a message or completion containing supported interleaved blocks and metadata is serialized and decoded
- **THEN** its block order, call identities, values, metadata and absent-versus-zero usage are preserved.

#### Scenario: Legacy message object

- **WHEN** a supported legacy message object contains a role and string content, including JSON-looking text
- **THEN** it decodes to that role and an exact text block without interpreting the string as structured content.

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
