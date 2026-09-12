## ADDED Requirements

### Requirement: Bounded provider failure diagnostics

Responses completion and model-catalog requests, Responses HTTP-success error
envelopes, and native ChatGPT failed stream events SHALL produce one finite,
Kuru-owned diagnostic class. The application MUST render only fixed Kuru text
and an HTTP status where present; it MUST NOT render provider message, parameter,
header, unknown code/type, endpoint URL/query, or other remote diagnostic text.

Provider failed-body inspection MUST be limited to 8 KiB and a two-second total
reader deadline within the existing operation deadline. A malformed, oversized,
stalled, or dribbling body MUST retain its fixed status-derived diagnostic. The
application MUST sanitize every error-chain element exposed to callers under
both Display and Debug formatting, including unexpected HTTP-success status
strings, and MUST
preserve the existing single rejected-401 rotation and no-replay-after-partial
completion rules.

#### Scenario: Known API model error is redacted
- **WHEN** a Responses completion receives HTTP 400 with `error.code` equal to
  `model_not_found` and a body message containing a fake secret
- **THEN** it reports the fixed selected-model availability/access diagnostic
  with HTTP 400 and no error-chain element contains the secret or model text

#### Scenario: HTTP-success error envelope is redacted
- **WHEN** a Responses completion returns HTTP 200 with a non-null error object
  whose message contains a fake secret
- **THEN** it reports a fixed provider protocol failure and no error-chain
  element contains the message

#### Scenario: Known native failed event is classified
- **WHEN** a successful ChatGPT SSE connection emits a terminal
  `response.failed` event with the observed `server_is_overloaded` code
- **THEN** it reports the fixed overloaded-service diagnostic without accepting
  or replaying the completion

#### Scenario: Unknown native failed event remains bounded
- **WHEN** a successful ChatGPT SSE connection emits a terminal failed event
  with an unrecognized code or type and a fake-secret message
- **THEN** it reports the fixed stream/protocol failure without rendering the
  code, type, message, or secret and without replaying the completion

#### Scenario: Dribbling failed body times out within the operation budget
- **WHEN** a non-success provider response dribbles its diagnostic body beyond
  the two-second diagnostic-reader deadline
- **THEN** it returns the fixed status-derived diagnostic before the containing
  catalog or completion deadline expires
