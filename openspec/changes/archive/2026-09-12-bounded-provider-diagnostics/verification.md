## 1. Bounded redacted Responses diagnostics [critical]

- [x] 1.1 @regression (agent) test HTTP-success envelopes through decoder and real HTTP provider paths -> the pre-fix completion decoder exposed `http-200-message-secret` in both Display and Debug; the final real-HTTP `response_envelopes_do_not_render_remote_values` test passed for error.message and unexpected status, with fixed failure and no remote values in either chain format
- [x] 1.2 @integration (agent) run completion/catalog HTTP diagnostics and source-gating fixtures -> finite/redacted completion cases, actual model-catalog 404/model_not_found, and API-versus-native classification tests passed; model/quota/rate/service wording excluded fake body values and API billing mappings did not cross into ChatGPT
- [x] 1.3 @integration (agent) run stalled/dribbling, chunked oversize and enclosing-deadline HTTP fixtures -> all passed: two-second total reader deadline survived timely dribbles, an unknown-length over-8-KiB quota body used status-only rate-limit fallback, and a 150-ms completion deadline preempted the reader within the test's 500-ms bound
- [x] 1.4 @regression (agent) run request-send and parser/environment chain fixtures -> closed-port query-redaction and parser/environment tests passed under full Display and Debug; invalid configured names, actual invalid-byte VarError payload and malformed function arguments were absent, with fixed typed transport wording

## 2. Native ChatGPT failed-stream behavior [critical]

- [x] 2.1 @integration (agent) run known native failed-event fixture -> `native_failed_events_and_initial_statuses_use_fixed_diagnostics` passed, reporting fixed overload text without remote message or accepted completion
- [x] 2.2 @regression (agent) run unknown native failure and existing partial-stream no-replay fixtures -> unknown code/type/message stayed absent from Display and Debug and request counts matched one request per invocation; the separate existing partial-output failure fixture proved no replay before a later explicit request

## 3. Scope preservation

- [x] 3.1 @integration (agent) run auth/provider budget and pending-context regressions -> API completion deadline/budget, repeated-401/no-partial-replay, replacement-login fence, malformed-SSE and pending-output retry cases passed; authentication routes and single rejected-401 rotation were preserved
- [x] 3.2 @manual (agent) inspect protocol and curated configuration references -> root reviewed both deltas; they describe finite bounded/redacted diagnostics and no replay. Owning docs check passed format, lint, VitePress build and public artifact/link/anchor checks without route or retry promises

## Observed evidence

2026-09-12, native macOS: focused provider evidence is retained in private logs
`/private/tmp/kuru-provider-diagnostics-*.log` (core, bodies, chains, envelopes,
deadline, operation-budget, native, rotation, sse-parser and pending). The owner
reported explicit exit 0 for those tasks; root checked their named passing
results. The pre-fix red observation was at the completion decoder sink; the
final HTTP-200 regression exercises the real local HTTP/provider path.

Connector lint/typecheck and Rust format checks passed. After the existing
embedded-runtime test adopted the new fixed 403 text, TUI all-target/all-feature
typecheck passed. `mise run //apps/kuru-docs:check` passed via its actual tool
result (no separate file log retained). No live provider or user credential
store was inspected; HTTP/SSE fixtures and environment sentinels are isolated.

The single final `mise run coverage` passed on native macOS with an explicit
process exit 0 (retained session 91931), including embedded-runtime behavior.
The log is `/private/tmp/kuru-phase0-provider-diagnostics-coverage.log`; it ends
with the completed LCOV report and 231.85-second task duration. Root independently
checked the fresh report: 17,781/18,415 lines, 96.56 percent, including the new
provider diagnostics module. The same run passed the new real-Dolt transaction
and isolated-publication cases. No duplicate behavioral suite or coverage writer
was started. Native Windows execution is unrun locally and remains a CI check
before merge.
