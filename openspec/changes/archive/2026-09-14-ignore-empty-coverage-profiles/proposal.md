## Why

The final Windows connector shard completed all selected tests, but one
instrumented process also left a zero-byte `.profraw` file. The coverage helper
rejected that file before writing a receipt even though the pinned LLVM profile
merger treats an empty profile as contributing no coverage, so the aggregate
correctly failed closed on an avoidable missing receipt.

## What Changes

- Validate and bound every raw-profile candidate before filtering zero-byte
  regular files from the shard receipt and copied evidence.
- Require each shard to retain at least one nonempty profile, while preserving
  candidate-count and total-byte limits before the zero-byte filter.
- Keep malformed nonempty profiles so the pinned LLVM reporter rejects them;
  preserve receipt hashing, aggregate provenance, exact execution ledgers, and
  the unchanged 90% line gate.
- Add focused fixtures for mixed empty/nonempty input, empty-only input,
  candidate limits, nonregular entries, and malformed nonempty data.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only `packages/kuru-delivery/src/coverage.rs` and its focused tests change. No
workflow topology, coverage schema, shard assignment, retry, dependency,
profile-size limit, or production behavior changes; existing documentation
does not describe zero-byte profile handling and remains accurate.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
