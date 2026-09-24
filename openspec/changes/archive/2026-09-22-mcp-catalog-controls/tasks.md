## 1. Configuration contract

- [x] 1.1 Add default-compatible MCP enabled, bounded original-name allow/deny glob, and HTTP header-environment-reference types and native validation; verify focused core tests cover defaults, deny precedence, bounds, invalid names, reserved headers, and stdio rejection.
- [x] 1.2 Update the published configuration schema and parser/schema fixtures; verify documented accepted examples pass both paths and every rejected control fails both paths with value-free diagnostics.

## 2. Private catalog metadata

- [x] 2.1 Implement the connector-owned owner-private versioned per-alias catalog store with workspace/authority/config/auth-context binding and fixed record/tool/field bounds; verify native store tests reject substitution, wrong privacy, context mismatch, wrong version, malformed bytes, and every limit.
- [x] 2.2 Integrate cache read and checked replacement with live discovery; verify an offline restart projects stale metadata without a route, partial publication is invisible, cache-write failure preserves a live route, and a healthy rediscovery repairs corrupt cache state.

## 3. MCP activation and transport

- [x] 3.1 Compile alias activation/filter policy before client construction and filter original names before routes/specs/cache; verify disabled launch/connect witnesses remain untouched and allow/deny overlap, nonempty allow, hostile discovery, and mixed alias health produce the exact catalog.
- [x] 3.2 Resolve HTTP static headers only from configured environment references and apply them through the single request boundary; verify local Streamable HTTP fixtures capture the header on initialize, list, call, protocol-version, and DELETE while missing/invalid references dispatch nothing and leak no value.
- [x] 3.3 Keep routes live-only and permission decisions authoritative; verify stale, disabled, denied, and guessed tools fail before dispatch while an admitted live tool resolves the existing alias/original selector and executes exactly once.

## 4. Shared inspection surfaces

- [x] 4.1 Extend `ToolCatalog`, MCP status, and `kuru tools` with the bounded disabled/live/stale/degraded projection; verify connector mixed-state projection and the shared CLI projection without a provider request.
- [x] 4.2 Register working `/tools` help/completion/parse/dispatch through the shared command registry; verify focused UI tests and one real deterministic PTY completed frame agree with `kuru tools` and expose no header value; record the exact mixed-state cross-surface probe as deferred integration evidence.

## 5. Documentation

- [x] 5.1 Update configuration, tools, usage, and public reference documentation with filter precedence, cache/status meaning, header references, permission separation, and explicit P13/Phase 4 exclusions; verify the owning docs format, build, content, and link checks.

## 6. Integrated acceptance and closeout

- [x] 6.1 Run the focused mixed-alias, offline restart, corrupt-repair, header lifecycle, permission, and CLI/TUI fixtures; record only observed outcomes and the missing composite provider-eval fixture in `verification.md`.
- [x] 6.2 Run connector/core/runtime/TUI all-target lint and typecheck, strict Cospec validation, and `git diff --check`; record combined coverage and hosted native checks as Delivery-owned post-commit gates.
- [x] 6.3 Archive `mcp-catalog-controls`, confirm the dated archive and merged living specs exist, and verify strict all-spec validation before the final conventional commit.
