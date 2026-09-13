# Dependencies

## Blocked by

None.

## Soft-blocked by

- [x] `tool-result-redaction` — provides the reviewed scanner and existing
  marker-safe runtime truncation helper at
  `packages/kuru-connectors/src/redaction.rs`. *(archived 2026-09-13)*
- [x] `owned-unix-shell-lifecycle` — provides the retained Unix shell capture
  path at `packages/kuru-connectors/src/unix_shell.rs`. *(archived 2026-09-13)*

## Siblings

The separately delivered MCP degradation work is excluded: this fix preserves
existing MCP stdio, HTTP JSON, and SSE framing/parser limits and changes no MCP
source.
