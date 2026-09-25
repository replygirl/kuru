## Why

Configured MCP servers currently start whenever their catalog is requested, publish every valid discovered tool, and lose all discovery metadata when a process exits. Users cannot disable one server, constrain its catalog before provider exposure, inspect a bounded stale catalog during an outage, or supply an HTTP credential without placing a literal value outside Kuru's established environment-reference boundary.

Phase 2 requires catalog controls that remain separate from execution authority: cached metadata may help inspection and provider planning, but it must never revive a disabled or denied route or claim that an unavailable server is live.

## What Changes

- Add per-server `enabled` controls and bounded tool-name allow/deny globs. Disabled servers never spawn or connect; deny takes precedence and a non-empty allow set admits only matching original server tool names.
- Add a checked, bounded private MCP catalog cache keyed by the configured server and its reviewed workspace/authority context. Valid stale metadata remains inspectable across restart, while execution still requires a current enabled live route. Corrupt or incompatible cache state degrades that alias and does not block a later live rediscovery from replacing it.
- Add HTTP MCP static headers whose values are resolved only through named environment references. Apply them to initialize, discovery, call, protocol response, and session-close requests without placing secret values in config projection, status, diagnostics, errors, or caches.
- Complete the shared-registry `/tools` surface with per-alias live/stale/degraded/disabled status and the filtered catalog. Keep `kuru tools` on the same projection.
- Preserve current permission evaluation at execution. Catalog inclusion is neither an execution grant nor evidence of availability.

## Capabilities

### New Capabilities

- `mcp-catalog-controls`: Enabled-state admission, filtered discovery, private stale catalogs, recoverable degradation, and environment-referenced HTTP headers.

### Modified Capabilities

- `configuration-schema`: Add and validate MCP enabled flags, bounded tool filters, and environment-referenced static header declarations with redacted projection.
- `provider-tools`: Filter MCP tools before provider exposure and require a current enabled live route plus the existing permission decision at execution.
- `command-registry`: Make `/tools` use the shared registry and project filtered MCP catalogs and per-alias status.
- `public-documentation`: Document MCP catalog controls, cache/status meaning, static-header references, and their security boundary.

## Impact

Primary source changes are in `packages/kuru-core/src/config.rs`, generated configuration schema fixtures, `packages/kuru-connectors/src/mcp.rs` and `tools.rs`, and the shared CLI/TUI command registry under `apps/kuru-tui`. A small private connector-owned catalog store is injected from the application data directory and remains outside the workspace tool root and Dolt. User and public documentation change with the schema and commands.

The change is source-stacked on archived P06 because both changes own `packages/kuru-connectors/src/tools.rs`; it does not change P06 scheduling semantics. It adds no OAuth, prompts/resources/sampling, provider credentials, general MCP session control plane, or product default model change.

## Surfaces

- [x] interactive — `/tools`, `kuru tools`, status and configuration diagnostics change
- [ ] deploy — no deployment topology, bind address or CI secret changes
- [x] integration — MCP discovery/call transport and HTTP header contracts change
- [x] agent-behavior — the provider-visible tool catalog is filtered and may contain explicitly stale metadata
