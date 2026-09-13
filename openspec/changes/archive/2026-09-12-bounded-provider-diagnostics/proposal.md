## Why

Provider failures currently reduce a non-success response to its status, while
ResponsesProvider::completion() formats provider-supplied error.message and
unexpected status values from HTTP-200 JSON envelopes directly. These can leak prompt, credential,
model, or endpoint context into the complete anyhow error chain, and raw
reqwest send errors can similarly retain configured query values.

Native ChatGPT SSE failed events also need bounded useful diagnostics without
assuming their private event schema is stable. Reading a failed provider body
must be limited in both bytes and time so a stalled or dribbling diagnostic
body cannot postpone a status failure until the containing catalog or
completion deadline.

## What Changes

- Classify Responses completion/catalog failures, Responses HTTP-200 error
envelopes, and native ChatGPT failed SSE events with a finite, Kuru-owned
diagnostic taxonomy. Render fixed text and status only; unknown provider strings
remain unrendered.
- Limit provider-only failed-body inspection to 8 KiB and a two-second total
diagnostic deadline within the existing 60-second catalog or 600-second
completion deadline. Preserve status-derived failure on malformed, oversized,
stalled, or dribbling bodies.
- Sanitize provider transport and diagnostic errors through every anyhow chain,
including request URLs/query values and remote body fields, under both full
Display and Debug formatting.
- Document the resulting bounded/redacted provider-failure behavior in the
protocol and curated configuration references without changing authentication,
generic MCP/A2A HTTP behavior, retry/backoff, routes, public completion shapes,
rotation, or no-replay semantics.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- provider-tools: provider failures have finite, redacted diagnostics across
  supported Responses and ChatGPT transports.
- public-documentation: protocol and configuration references describe the
  bounded, redacted provider-failure behavior.

## Impact

packages/kuru-connectors/src/providers/diagnostics.rs, providers.rs, and
providers/sse.rs supply private classification and bounded-reader behavior;
their focused local HTTP/SSE fixture modules exercise it. The existing packaged
runtime test in apps/kuru-tui/tests/embedded_runtime.rs adopts the new fixed
403 diagnostic while retaining its offline/runtime assertions. docs/protocols.md and
apps/kuru-docs/reference/configuration.md state the user-visible contract. No
dependency, configuration, credential-store, provider-routing, public Rust/JSON
API, or retry-policy change is included.

## Surfaces

- [x] interactive — completion and catalog errors rendered in the terminal/CLI
- [ ] deploy — deploy/runtime/CI-execution topology
- [x] integration — Responses and native ChatGPT provider error contracts
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
