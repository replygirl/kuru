## Why

Tool results currently cross from connectors into direct CLI output and durable
runtime tool messages without one common recognizable-secret projection. Provider
diagnostics are already bounded, but successful file, shell and MCP content and
useful MCP application errors can still expose common credential forms.

Kuru needs a finite, documented filter at its tool-result authority boundary. It
must preserve ordinary output and current producer limits while stating clearly
that pattern recognition cannot detect arbitrary secrets.

## What Changes

- Add one connector-owned finite scanner for documented authorization headers,
  provider token heuristics, private-key blocks and contextual sensitive fields.
- Represent tool output and application errors privately as typed text or JSON,
  project them once before the existing public `Result<String>` boundary, and
  preserve valid JSON and safe MCP `isError` content.
- Prevent raw tool errors and their Display/Debug chains from bypassing the
  projection while retaining typed MCP failure authority for later degradation
  work.
- Preserve file inputs/writes, prior history, and each producer's existing
  capture/listing limits; document visible markers and known false negatives.
- Keep replacement markers whole when runtime tool receipts encounter their
  existing byte limits, using a connector-owned truncation helper without
  changing generic chat truncation or adding head/tail output.
- Add synthetic scanner, real file/shell/MCP, direct CLI and runtime-persistence
  evidence on supported native platforms.

## Capabilities

### New Capabilities

### Modified Capabilities

- `provider-tools`: Project recognized credential patterns from all tool results
  and outward tool errors through one typed, bounded connector boundary.
- `public-documentation`: Explain tool-result projection, its visible marker,
  preserved inputs and its finite-recognition limits.

## Impact

The change affects connector tool execution and private result/error types,
connector scanner and focused fixtures, narrow runtime tool-receipt truncation
call sites, direct CLI/runtime integration fixtures,
and curated tool/protocol documentation. `ToolHost::execute` retains its existing
public `Result<String>` signature. It adds no dependency, configuration, storage
migration, credential registry or public JSON field.
Connectors exposes a pure `truncate_tool_output` helper so the runtime does not
duplicate detector or replacement-marker policy.

## Surfaces

- [x] interactive — direct CLI and TUI/runtime-visible tool results are projected
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — stdio and HTTP MCP application results cross the boundary
- [x] agent-behavior — provider-facing and persisted tool messages receive the projection
