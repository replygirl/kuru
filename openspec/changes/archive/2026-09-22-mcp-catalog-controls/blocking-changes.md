# Dependencies

## Blocked by

- [x] `workspace-trust` — reviewed automatic MCP authority, retained exact workspace identity, and pre-activation trust gating *(archived 2026-09-12)*
- [x] `mcp-server-degradation` — alias-local MCP health, explicit recovery, and bounded diagnostics *(archived 2026-09-13)*
- [x] `tool-permission-decisions` — stable MCP selectors and the authoritative per-call permission evaluator *(archived 2026-09-16)*
- [x] `phase2-config-instructions` — typed configuration/provenance layers and strict published-schema parity *(archived 2026-09-22)*
- [x] `command-registry` — shared working TUI help, completion, parsing, and dispatch registry *(archived 2026-09-22)*
- [x] `parallel-independent-tool-execution` — current ToolHost boundary and serial treatment of opaque MCP calls *(archived 2026-09-22)*

## Soft-blocked by

None.

## Siblings

P08 provider-summary persistence owns provider/actor settlement and P27-based memory facade/RPC paths. It does not own MCP catalog or `tools.rs` source, so it is coordinated but not a blocker.
