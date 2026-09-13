## 1. Bounded producer excerpts [critical]

- [x] 1.1 @regression (agent) run the real oversized regular-file and independent shell-stream probes before and after the correction -> pre-fix source rejects retained output at 2 MiB; corrected source returns exact bounded head, omission marker, and tail after scanner projection while draining to EOF.
- [x] 1.2 @integration (agent) stream fake recognized-secret values across chunk and head/tail joins -> full redaction markers remain visible and no raw token or partial marker is retained.
- [x] 1.3 @integration (agent) exercise the existing typed file-list cap and oversized stdio/HTTP/SSE MCP fixtures -> file-list stays valid JSON without a new truncation schema; MCP wire framing/parser rejection remains unchanged.

## 2. Model and direct-command visibility [critical]

- [x] 2.1 @integration (agent) drive runtime tool results across the 8 KiB receipt and actor quota limits -> every receipt is valid JSON, keeps its call ID and a marked tail, and persists the projected output only to the speaking peer context.
- [x] 2.2 @e2e (agent) invoke an isolated direct `kuru tool` file or shell command -> stdout contains the exact marked bounded receipt with no provider or live credential access.
- [x] 2.3 @eval (agent) compare the next provider request after an oversized tool call with the exact captured receipt -> the speaking actor receives one valid call-ID-preserving head-and-tail excerpt and no raw recognized secret.

## 3. Quality and supported platforms

- [x] 3.1 @integration (agent) run focused connector/runtime/TUI tests, format, lint, typecheck, and documentation checks -> required package-owned checks pass with prepared fixtures.
- [x] 3.2 @integration (agent) run actual independent oversized stdout/stderr shell capture on native Windows -> hosted PR18 `ba4a3b7` Windows application job `103756331991` passed the native suite, including the authoritative shell-capture acceptance path.
- [x] 3.3 @integration (agent) run the coordinated workspace coverage writer -> PR18's coordinated normal hook passed all eight categories at `32886/35006 = 93.9439%` (`ba4a3b7`).

## Observed evidence

- Baseline `4de6b2a` passed its normal coverage in
  `/private/tmp/kuru-mcp-stacked-prepush-retry.log`; its
  `permissions_and_size_limits_reject_before_mutation`,
  `shell_returns_status_bounds_output_and_terminates_on_timeout`, and
  `shell_capture_rejects_overflow` probes establish the prior hard-limit
  rejection behavior.
- Corrected focused connector probes passed: marker-safe chunk/join and large
  append retention, oversized regular-file and independent Unix stream excerpts,
  invalid-byte shell rendering, typed file-list cap, and unchanged oversized RPC
  admission.
- Corrected runtime and direct CLI probes passed: 8 KiB and actor receipt limits,
  real fake-provider continuation/reopen with one private persisted receipt, and
  isolated `kuru tool file_read` output/source identity.
- `mise run //packages/kuru-connectors:typecheck`, connector lint, runtime and
  TUI typecheck/lint, `mise run format:rust:fix`, and `mise run docs:check`
  passed. Hosted PR18 `ba4a3b7` Windows full-application job `103756331991` then passed.

- 2026-09-13 local PR18 `ba4a3b7`: the coordinated normal hook passed all eight
  categories; its workspace coverage report measured `32886/35006 = 93.9439%`.

- 2026-09-13 hosted PR18 `ba4a3b7` run `34769483594`: Ubuntu, macOS, Windows platform, and Windows full-application jobs all passed; Windows platform job `103756331854` measured `3329/3693 = 90.1435%`, and application job `103756331991` completed successfully.
