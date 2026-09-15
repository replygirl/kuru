## MODIFIED Requirements

### Requirement: Recognizable-secret tool-result projection

Before returning any built-in, shell or MCP result through `ToolHost::execute`,
the connector SHALL project a finite documented set of recognizable credential
patterns to the exact marker `[REDACTED:recognized-secret]`. The projection MUST
cover successful text and typed JSON, useful MCP application-error content and
every outward tool error Display, alternate Display, Debug and source-chain path.
It MUST NOT claim recognition of arbitrary, encoded, split, transformed or future
secret formats.

The finite set MUST include Basic and Bearer Authorization and
Proxy-Authorization values; bounded local OpenAI, GitHub, Slack, GitLab and AWS
token heuristics; JWT-shaped three-segment values; URL userinfo for bounded
RFC-style schemes; supported private-key blocks; and exact contextual sensitive
field names, including bare `token` and `secret`. JWT recognition MUST require a
base64url JSON-object header with a nonempty `alg` field and three bounded
base64url segments; it is a detector, not credential validation. An overlong
pending recognizable candidate MUST project fail-closed rather than return raw
buffered text. In typed JSON, an exact contextual or Authorization
key MUST replace its complete associated value even when that value is non-string,
while other string keys and values replace only recognized spans. Typed JSON
MUST remain valid and MUST fail safely rather than overwrite members if projected
keys collide. Arbitrary text which happens to parse as JSON MUST remain text,
while bare and matching-quoted contextual/header assignment syntax is still
recognized.

The connector SHALL expose its text and JSON projection operations as pure
reusable APIs while preserving `ToolHost::execute`'s existing public
`Result<String>` interface. The projection scanner MUST remain bounded across
arbitrary byte chunking and replace a recognized span before retained truncation
can expose a fragment.

#### Scenario: Successful tool content is projected

- **WHEN** an allowed file read, native shell or successful stdio or HTTP MCP
  response contains a supported synthetic credential pattern
- **THEN** `ToolHost::execute` returns the visible marker in place of that
  pattern and returns no raw match through another error or formatting path

#### Scenario: Sensitive typed field is projected

- **WHEN** a typed JSON tool result contains an exact sensitive or Authorization
  key with a string, numeric, boolean, null, array or object value
- **THEN** its whole associated value is the marker, all unrelated structure
  remains semantically unchanged and the serialization is valid JSON

#### Scenario: Quoted contextual text is projected without JSON coercion

- **WHEN** arbitrary text contains a matching-quoted contextual or Authorization
  key and value in JSON-like syntax
- **THEN** only its recognized value span is replaced and the surrounding text
  is not parsed, reordered or reserialized

#### Scenario: Unrecognized ordinary output is preserved

- **WHEN** tool output contains ordinary source text, hashes, UUIDs, model
  names, generic base64, certificates or provider-like strings below documented
  local floors
- **THEN** its unmatched bytes remain identical and Kuru makes no claim that an
  unknown credential shape would be found

#### Scenario: Expanded recognizable forms are projected

- **WHEN** allowed tool text or typed JSON contains a qualifying Slack `xox*`,
  GitLab `glpat-`, JWT-shaped, URL-userinfo, `token=` or `secret=` synthetic value
- **THEN** the returned projection contains the visible marker and no raw matched
  span

#### Scenario: Ordinary near matches remain exact

- **WHEN** tool output contains ordinary dotted identifiers, non-userinfo URLs,
  `tokenize` or `secretary`, an invalid or short JWT-like value, or a short token
  prefix
- **THEN** unmatched bytes remain identical

#### Scenario: Added forms survive streaming boundaries

- **WHEN** each added recognizable form is split at every byte boundary or read
  one byte at a time
- **THEN** streaming projection equals whole-value projection and contains no
  raw matched fragment
