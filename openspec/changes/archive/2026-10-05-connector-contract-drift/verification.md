# Verification

## 1. Positive incompatibility classification [critical]

- [x] 1.1 @integration (agent) send complete successful auth/catalog responses with required fields omitted and complete terminal responses with required item/tool identity contradictions -> only those cases produce fixed typed incompatibility codes with bounded diagnostics and no remote body or credential text; observed token omission HTTP fixture, catalog omission and terminal contradiction, all fixed/sanitized
- [x] 1.2 @integration (agent) send timeouts, read failures, incomplete streams, rejected/expired credentials, rate limits, and malformed model-generated arguments -> existing operational or argument-validation outcomes remain, with no incompatibility code or route switch; observed HTTP rejection, expiry, account mismatch, malformed JSON, transport refusal, storage read failure, incomplete/oversized SSE and malformed arguments all untyped
- [x] 1.3 @integration (agent) pass a contradictory completed tool response through provider settlement -> the decoder returns a typed error before a completion can reach ordinary runtime dispatch; the bounded canary confirms catalog plus exactly one no-tool completion request and no redispatch. The canary itself does not construct a ToolHost
- [x] 1.4 @eval (agent) compare an accepted-versus-contradictory deterministic response corpus -> compatible terminal content, ordering and usage remain exact while only positively contradictory completed calls stop before a completion can reach runtime dispatch; observed 16 SSE decoder regressions and focused catalog/completion fixtures

## 2. Compatible provider evolution remains supported [critical]

- [x] 2.1 @integration (agent) replay accepted native authentication/refresh, catalog, item-ID stream, reasoning, usage, and tool fixtures with unknown harmless metadata/model/effort values and optional usage/reasoning absent -> accepted normalized content, route separation, known terminal shapes, and same-item tool reconciliation remain unchanged; observed catalog future model/effort metadata plus existing refresh and decoder regression fixtures
- [x] 2.2 @regression (agent) run the existing empty-terminal-envelope, terminal ordering, visible-text agreement, same-item identity, and retry/auth route tests -> accepted existing contracts pass without new fallback or ambiguous dispatch; observed 16 SSE decoder regression tests and scoped refresh/route fixtures

## 3. Explicit canary reports only observed stages [critical]

- [x] 3.1 @e2e (agent) invoke `kuru canary --model MODEL` in a fresh process against loopback fake subscription endpoints with synthetic Kuru credentials -> one bounded no-tool request uses only ChatGPT auth, emits sanitized schema-v1 JSON, and exits 0 for a compatible completed response; observed exact fresh-process fixture
- [x] 3.2 @e2e (agent) run the fresh-process canary for a complete incompatible response, unavailable credentials/transport, and ordinary CLI misuse -> exits are respectively 3, 2, and 1; no workspace configuration, memory, tool authority, API-key route, or raw provider body is observed; observed fresh-process fixtures
- [~] 3.3 @manual (human) run the explicit canary only if an operator with a participating account volunteers -> defer: no participating account was offered, so no live account call was made; live canary remains optional and unverified, with no CI or release gate

## 4. Scoped delivery checks

- [x] 4.1 @regression (agent) run owning connector and TUI format/lint/typecheck, focused deterministic fake-service tests, docs checks, strict/managed Cospec checks -> all local scoped checks pass; normal commit hooks run with the final commit and are reported separately
- [~] 4.2 @runtime (agent) after archive and PR integration, run integrated-head workspace coverage and supported-platform native checks -> defer: these mandatory hosted checks follow archive and PR creation; coverage must remain at least 90%, and native results must be recorded separately from local fake-service evidence before full-goal delivery is complete
