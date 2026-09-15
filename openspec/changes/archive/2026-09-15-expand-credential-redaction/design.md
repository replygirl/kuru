## Context

The existing scanner is a bounded byte-stream state machine. It deliberately
recognizes only finite forms so ordinary source and tool output survives exactly,
but its current inventory omits several common credential presentations.

## Goals / Non-Goals

**Goals:**

- Recognize the approved finite forms in text, JSON keys and JSON values while
  preserving chunk-invariant bounded projection.
- Reuse the connector projection policy from later runtime event redaction.

**Non-Goals:**

- Detect high-entropy, encoded, encrypted, unknown, or future credentials.
- Read credential stores, inspect environment variables, alter execution input,
  or change files and durable history.

## Decisions

- Extend the existing scanner states and prefix matcher rather than add a
  post-processing regex. This preserves streaming behavior, output bounds and
  marker atomicity.
- Recognize Slack `xox` forms and GitLab `glpat-` through explicit bounded
  prefix alphabets and minimum lengths. They are format recognition, not
  validation of live credentials.
- Recognize JWTs only after a base64url JSON-object header and two further
  bounded base64url segments. `typ` is optional as accepted for this change;
  the object-header check avoids treating arbitrary dotted source text as a JWT.
- Recognize URL credentials for bounded RFC-style
  `[A-Za-z][A-Za-z0-9+.-]*://user:password@host` syntax, replacing the userinfo
  span while retaining the URL location. URLs without userinfo remain exact; an
  authority candidate that exceeds its bounded buffer projects fail-closed
  rather than exposing a potentially long password. Long scheme names stream
  after their bounded prefix while retaining enough state to recognize `://`.
- Add bare `token` and `secret` to the exact contextual assignment names. Word
  boundaries retain `tokenize` and `secretary` as ordinary text.
- Export only pure text/JSON projection operations from `kuru-connectors`; the
  existing `ToolHost` result shape stays unchanged.

## Risks / Trade-offs

- Broader lexical detection can hide ordinary output → test exact near matches,
  documented floors and malformed controls.
- JWT header parsing can complicate bounded streaming state → cap each pending
  segment and flush invalid candidates as exact text.
- Public projection APIs can invite callers to skip the tool boundary → expose
  only the existing projection semantics and keep tool execution authority
  private.
