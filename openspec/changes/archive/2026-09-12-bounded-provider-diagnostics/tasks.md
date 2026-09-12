## 1. Provider diagnostic boundary

- [x] 1.1 Add the provider-only finite classifier and 8 KiB/two-second failed-response reader within existing operation budgets, preserving generic HTTP users
- [x] 1.2 Route Responses completion/catalog failures, HTTP-200 error envelopes and unexpected status strings, native initial status failures, and supported native failed SSE events through fixed redacted diagnostics
- [x] 1.3 Replace reqwest/provider diagnostic chains exposed by provider paths with fixed typed errors, retain typed transport classification for future policy, and explicitly prohibit inferring an unsent request or authorizing replay from it

## 2. Regression coverage and documentation

- [x] 2.1 Add local HTTP/SSE regressions that fail before the fix and pass after for HTTP-200 message redaction, bounded status fallback, known and unknown native failed events, and no replay
- [x] 2.2 Add full-anyhow-chain Display and Debug redaction regressions with fake body, unexpected status, and query secrets
- [x] 2.3 Update docs/protocols.md and apps/kuru-docs/reference/configuration.md with the bounded/redacted diagnostic contract

## 3. Verification

- [x] 3.1 Run the focused connector fixtures and record actual evidence in verification.md
- [x] 3.2 Run required formatting/static/docs checks with the coordinated integration batch and record actual or deferred evidence
