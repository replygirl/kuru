## 1. MCP cache fixture

- [x] 1.1 Update `packages/kuru-connectors/src/mcp_cache.rs` to preserve Unix checked-root substitution refusal, prove Windows Pinned rename denial with unchanged root identity and bytes, and retain non-object schema rejection.
- [x] 1.2 Run the focused fixture on the available native host, owning connector lint, format and strict Cospec checks; record Windows as a required final-head CI gate rather than a local pass.

Observed on macOS: the exact cache-root fixture passed 1/1; owning connector all-target Clippy passed. Windows Pinned rename denial and retained identity/bytes are source-reviewed and remain unrun until final-head native CI.
