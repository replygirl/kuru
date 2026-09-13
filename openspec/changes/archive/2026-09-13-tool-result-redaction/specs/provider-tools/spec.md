## ADDED Requirements

### Requirement: Recognizable-secret tool-result projection

Before returning any built-in, shell or MCP result through `ToolHost::execute`,
the connector SHALL project a finite documented set of recognizable credential
patterns to the exact marker `[REDACTED:recognized-secret]`. The projection MUST
cover successful text and typed JSON, useful MCP application-error content and
every outward tool error Display, alternate Display, Debug and source-chain path.
It MUST NOT claim recognition of arbitrary, encoded, split, transformed or future
secret formats.

The finite set MUST include Basic and Bearer Authorization and
Proxy-Authorization values; bounded local OpenAI, GitHub and AWS token heuristics;
supported private-key blocks; and exact contextual sensitive field names. In
typed JSON, an exact contextual or Authorization key MUST replace its complete
associated value even when that value is non-string, while other string keys and
values replace only recognized spans. Typed JSON MUST remain valid and MUST fail
safely rather than overwrite members if projected keys collide. Arbitrary text
which happens to parse as JSON MUST remain text, while bare and matching-quoted
contextual/header assignment syntax is still recognized.

#### Scenario: Successful tool content is projected

- **WHEN** an allowed file read, native shell result or successful stdio or HTTP MCP response contains a supported synthetic credential pattern
- **THEN** `ToolHost::execute` returns the visible marker in place of that pattern and returns no raw match through another error or formatting path

#### Scenario: Sensitive typed field is projected

- **WHEN** a typed JSON tool result contains an exact sensitive or Authorization key with a string, numeric, boolean, null, array or object value
- **THEN** its whole associated value is the marker, all unrelated structure remains semantically unchanged and the serialization is valid JSON

#### Scenario: Quoted contextual text is projected without JSON coercion

- **WHEN** arbitrary text contains a matching-quoted contextual or Authorization key and value in JSON-like syntax
- **THEN** only its recognized value span is replaced and the surrounding text is not parsed, reordered or reserialized

#### Scenario: Unrecognized ordinary output is preserved

- **WHEN** tool output contains ordinary source text, hashes, UUIDs, model names, generic base64 or JWT-like values, certificates or provider-like strings below the documented local floors
- **THEN** its unmatched bytes remain identical and Kuru makes no claim that an unknown credential shape would be found

### Requirement: Typed tool output and application errors

Tool implementations SHALL retain a private text-versus-JSON result type through
projection while preserving the public `Result<String>` interface. MCP
`isError=true` content MUST remain a useful projected application error and MUST
NOT be reclassified as transport unavailability. Tool transport and protocol
failures MUST retain a typed safe category without parsing formatted text, and
the outward error MUST NOT retain a raw source-chain bypass.

#### Scenario: MCP application error remains useful

- **WHEN** a healthy MCP server returns `isError=true` with ordinary instructions and a supported synthetic credential
- **THEN** the caller receives the useful instructions with the credential replaced and the server is not marked unavailable

#### Scenario: Raw remote failure cannot escape through formatting

- **WHEN** an MCP transport or protocol error contains a supported synthetic credential in its remote detail
- **THEN** Display, alternate Display, Debug and complete source-chain formatting expose only the typed safe category and projected detail

### Requirement: Projection preserves authority and producer bounds

Tool-result projection MUST NOT change tool arguments, file writes, remote
requests, earlier durable history, provider credential stores or environment
selection. It SHALL preserve the existing per-producer limits: 2 MiB file reads,
10,000-entry complete file listings, independently bounded 2 MiB shell stdout and
stderr fields, and bounded MCP messages/responses. It MUST NOT impose a new
global 2 MiB result limit. Projection allocation and runtime MUST remain bounded
relative to already-admitted producer input, and redaction MUST occur before any
truncation or human terminal escaping.

#### Scenario: Write input remains exact

- **WHEN** an authorized file write receives content matching a supported detector
- **THEN** the exact requested bytes are written and only a later returned projection is eligible for filtering

#### Scenario: Combined shell output keeps its current allowance

- **WHEN** shell stdout and stderr each contain an allowed result near their independent existing limits
- **THEN** the complete typed shell result remains successful and projection does not reject it under a new combined 2 MiB limit

#### Scenario: Chunk and EOF boundaries do not leak

- **WHEN** a supported token, contextual value or private-key block crosses any scanner chunk boundary or ends incompletely at EOF
- **THEN** streaming and whole-value projection agree and no recognized fragment is exposed by truncation or finalization

### Requirement: Projected runtime tool authority

Only the projected result or safe projected error SHALL cross from ToolHost into
direct CLI output or runtime tool forwarding. Runtime persistence and subsequent
provider prompts MUST use that same projected value, while prior history and the
tool's source or side-effect data remain unchanged.

Existing context byte limits MAY omit projected content. Tool-specific
truncation MUST keep retained replacement markers whole and MUST NOT bisect an
earlier marker when shortening a prefix to fit one. If a complete marker cannot
fit the available budget, it MUST be wholly omitted with the existing bounded
truncation indication. Generic chat truncation and legacy tool-history parsing
semantics MUST remain unchanged.

#### Scenario: Runtime persists the projection

- **WHEN** a real tool turn returns a supported synthetic credential followed by an ordinary completion
- **THEN** the persisted tool message and next provider-facing context contain the same marker, existing activity remains metadata-only, and none contains the raw credential

#### Scenario: Context limit intersects a replacement marker

- **WHEN** a current tool output or older tool message reaches an existing byte limit inside a replacement marker
- **THEN** the retained marker is complete when it fits, otherwise it is wholly omitted with bounded truncation indication; UTF-8 and existing byte limits remain valid, and arbitrary legacy tool text gains no new parsing requirement
