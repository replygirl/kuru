## 1. Finite scanner behavior [critical]

- [x] 1.1 @unit (agent) run table-driven whole-value and every-byte chunk-split cases for all detector prefixes, boundaries, floors, quotes, private-key delimiters, overlap and EOF states -> `mise run coverage` exited 0; the instrumented scanner suite observed the exact marker, unchanged unmatched bytes and AWS boundary behavior (2026-09-12).
- [x] 1.2 @unit (agent) run documented false-positive and false-negative controls for hashes, UUIDs, model names, base64/JWT-like text, certificates, short provider prefixes and unknown/transformed shapes -> `mise run coverage` exited 0; scanner controls retained documented unmatched shapes and make no arbitrary-secret claim (2026-09-12).
- [x] 1.3 @integration (agent) run real allowed file-read and native shell-output fixtures with supported fake credentials and ordinary controls -> `mise run coverage` exited 0; file and Unix shell projection fixtures observed markers, ordinary controls and unchanged source/side-effect bytes (2026-09-12).

## 2. Typed structured output [critical]

- [x] 2.1 @unit (agent) project nested typed JSON containing exact sensitive and Authorization keys with string, numeric, boolean, null, array and object values plus recognized spans inside ordinary keys -> `mise run coverage` exited 0; nested structured projection tests observed whole selected markers, semantic siblings and valid JSON (2026-09-12).
- [x] 2.2 @unit (agent) project colliding keys and arbitrary JSON-looking text with bare/quoted contextual and Authorization assignments -> `mise run coverage` exited 0; collision and arbitrary-text controls produced the fixed withheld result or unchanged ordering as applicable (2026-09-12).
- [x] 2.3 @integration (agent) execute fake stdio and HTTP MCP success and `isError` responses containing nested fake credentials -> focused HTTP projection passed and `mise run coverage` exited 0; useful projected content remained available and raw fake credentials were absent (2026-09-12).

## 3. Safe failure boundary [critical]

- [x] 3.1 @integration (agent) execute file and shell failures with synthetic credential-bearing input/capture, and MCP protocol failures with credential-bearing actionable detail -> focused Unix timeout fixture and `mise run coverage` exited 0; Display, alternate, Debug and complete source chains retained fixed/projected output only, with no raw fake credential or shell capture (2026-09-12).
- [x] 3.2 @regression (agent) exercise the existing provider-failure, tool validation and MCP timeout classifications -> `mise run coverage` exited 0; regression classifications retained fixed diagnostics and typed MCP causes (2026-09-12).

## 4. CLI and runtime authority [critical]

- [x] 4.1 @e2e (agent) run the real direct `kuru tool` path for file, native shell and fake MCP results -> `mise run coverage` exited 0 and exercised the direct CLI fixtures; visible projection omitted fake raw credentials while ordinary controls remained exact (2026-09-12).
- [x] 4.2 @runtime (agent) complete a real tool turn with a fake credential result, persist it, inspect metadata-only activity, the next provider-facing context and a reopened session, then exercise marker cuts at current and optional tool-receipt limits with UTF-8, adjacent markers and tiny budgets -> `mise run coverage` exited 0; runtime receipt/persistence/reopen/truncation fixtures observed whole markers, bounded output and unchanged earlier data (2026-09-12).
- [~] 4.3 @e2e (agent) run 4.1 and 4.2 on native Windows with the supported PowerShell/process and real-memory fixtures -> defer: supported native Windows CI was not run locally; compilation and macOS coverage are not parity evidence.
- [x] 4.4 @eval (agent) inspect captured provider requests from a real tool-result continuation containing recognized and ordinary controls -> `mise run coverage` exited 0; captured-request runtime fixtures observed absent recognized credentials, preserved marker positions and exact ordinary controls (2026-09-12).

## 5. Existing budgets and bounded processing [critical]

- [x] 5.1 @integration (agent) return independently near-limit shell stdout and stderr, a complete listing serialized above 2 MiB and at-limit file/MCP content -> focused exact-wire HTTP and `mise run coverage` exited 0; independent caps and legitimate unmatched results remained successful without a global result limit (2026-09-12).
- [x] 5.2 @unit (agent) exercise adversarial producer-limit input, maximum marker expansion, overflow checks, long whitespace/indentation/private-key bodies, repeated small serialization writes and forced expansion-bound breach -> `mise run coverage` exited 0; bounded scanner allocation and withheld-overflow tests passed (2026-09-12).
- [x] 5.3 @unit (agent) feed synthetic stderr-like bytes across every chunk/tail boundary and close at complete/incomplete EOF -> `mise run coverage` exited 0; synthetic scanner chunk/tail/EOF tests passed without making an MCP stderr-capture claim (2026-09-12).

## 6. Public explanation

- [x] 6.1 @integration (agent) build and check the curated documentation -> `mise run docs:build` and `mise run docs:check` exited 0; curated tools/protocol pages contain the public guidance without publishing private design notes (2026-09-12).
