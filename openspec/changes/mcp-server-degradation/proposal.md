## Why

One unavailable MCP server currently aborts the complete tool catalog, hiding built-in and healthy-server tools and preventing the turn from continuing. Stdio server stderr is discarded, while its process owner can reuse an ambiguously cancelled JSON-RPC session and can signal a numeric Unix process group after reaping its root.

Phase 0 needs honest alias-local degradation and useful diagnostics without adding retries, background health machinery, or another process framework. The implementation can build on Kuru's existing typed tool projection, streaming secret scanner, Unix process-group owner, Windows Job owner, and runtime event shape.

## What Changes

- Discover, validate, publish, disable, and explicitly recover MCP tools per configured alias so one failed alias does not remove built-in or healthy-server tools.
- Serialize each alias's complete catalog operation and calls, disable an ambiguous session without replay, and reject new work after host shutdown begins.
- Replace stdio RPC's caller-owned child/numeric group with a connector-private worker retaining the platform owner and all pipes through bounded cleanup.
- Continuously drain stderr, redact recognizable secrets before bounded retention, and expose only a terminal-safe human diagnostic; runtime events, prompts, tool errors, and memory receive fixed metadata.
- Preserve MCP application `isError` as a healthy application result, current tool/result stdout shapes, explicit workspace authority, and immediate pre-spawn retained-root revalidation.

## Capabilities

### New Capabilities

### Modified Capabilities

- `provider-tools`: Define alias-local MCP degradation, explicit recovery, no replay after ambiguous dispatch, fixed runtime status, human-only bounded stderr diagnostics, and closed host admission.
- `native-platform`: Extend the retained Unix process-group owner with one-shot stdin extraction for an independently owned stdio session.
- `public-documentation`: Document MCP availability, recovery, diagnostic redaction and stderr visibility without claiming sandboxing or arbitrary-secret detection.

## Impact

This changes the connector tool catalog API, MCP client/session internals, runtime catalog consumption, direct CLI status presentation, one narrow Unix platform method, MCP fixtures, and protocol/tools documentation. It uses the workspace-pinned futures crate without changing its version. There is no configuration, persistence, database, provider, or release change, and existing `ToolHost::specs()` remains compatible.

## Surfaces

- [x] interactive — unavailable MCP aliases and safe human diagnostics are visible while healthy tools remain usable.
- [ ] deploy — no deploy, workflow, secret, or runtime-topology change.
- [x] integration — stdio and Streamable HTTP MCP session and catalog behavior changes.
- [x] agent-behavior — the provider receives only currently usable tools and fixed failed-server activity remains outside prompts and memory.
