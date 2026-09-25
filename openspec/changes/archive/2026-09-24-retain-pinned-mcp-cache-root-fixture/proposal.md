## Why

The MCP cache root-substitution fixture assumes a retained directory can be renamed on Windows. The existing Pinned directory contract now correctly rejects that operation, so the fixture must prove the same checked-root behavior in a platform-appropriate way.

## What Changes

- Update the MCP cache root fixture to keep its Unix replacement refusal and to assert Windows sharing denial, unchanged retained identity and bytes, and no replacement root.
- Preserve the independent non-object schema rejection and cache authority checks.

## Impact

Only `packages/kuru-connectors/src/mcp_cache.rs` tests change. The focused fixture needs native Windows verification; no product API or cache behavior changes.
