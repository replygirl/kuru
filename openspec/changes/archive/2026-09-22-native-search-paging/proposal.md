## Why

Kuru's checked workspace tools can list and read files, but cannot search a
project or continue a large text file from a precise point. That makes ordinary
repository investigation unnecessarily expensive and encourages a model to use
shell for a read-only operation that should stay within the file capability.

Phase 2 parity requires native grep and glob without a separately installed
`rg`, plus predictable file-read pages. These operations need the same checked
root, permission, redaction, and bounded-output guarantees as the existing
native file tools.

## What Changes

- Add native `grep` and `glob` tools backed by pinned upstream Rust search
  components, with bounded traversal, matching, result output, and no external
  executable requirement.
- Add optional line-based `offset` and `limit` paging to `file_read`, including
  additive continuation and omission metadata while retaining the unpaged text
  result.
- Extend native tool and permission configuration schemas for search tools,
  preserving per-target denials during traversal.
- Document tool inputs, hidden/ignore defaults, paging units, continuations,
  binary handling, and bounds; include upstream license material for shipped
  search components.

## Capabilities

### New Capabilities
- `native-search-paging`: Checked native project search, globbing, and paged
  text-file reading.

### Modified Capabilities
- `provider-tools`: Native tool catalog, permission evaluation, and projected
  output contracts.
- `configuration-schema`: Native permission selector/schema values for search
  tools.
- `public-documentation`: User-facing built-in-tool reference.

## Impact

- `packages/kuru-connectors`: search traversal, native tool dispatch, pinned
  Rust dependencies, notices, and tests.
- `packages/kuru-core`: native tool enum/schema and permission matching tests.
- `docs/protocols.md`: native tool and paging documentation.
- `Cargo.toml` and `Cargo.lock`: exact workspace dependency pins for upstream
  Rust search crates.

## Surfaces

- [ ] interactive — no TUI surface is added.
- [ ] deploy — no deployment topology changes.
- [x] integration — pinned upstream Rust search components and notices.
- [x] agent-behavior — native tool definitions, permission decisions, and
  tool-result shape change.
