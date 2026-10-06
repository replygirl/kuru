# Proposal

## Why

PR249 at 94ad967e failed three memory fixtures across Ubuntu ARM partitions.
The driver fixture still expects the removed outer "draining work" diagnostic,
and the empty-scope and WAL legacy fixtures open fresh cold stores without
warming their preparation cache. The official jobs 112520785435 and
112520785562 observe these failures; the former does not preserve the driver
refusal's original error value.

## What Changes

Assert the existing typed native maintenance contention error with full-chain
failure diagnostics. Restore warm preparation for the empty-scope and WAL
fixtures, retaining their custom roots and legacy data. Both produce a prepared
legacy import, which the existing creation selector routes through cold creation
before template selection; warming does not substitute a template-backed store.
All ownership, durable-intent, hold-release, migration and cleanup assertions
remain in place. Production APIs, deadlines and behavior do not change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing ownership and cold migration contracts remain correct.

## Impact

Only the named fixtures in packages/kuru-memory/src/facade/driver.rs and
packages/kuru-memory/src/migration.rs change. No dependencies or public APIs.

Acceptance uses the three original named cases and the existing closing guard
through the owning memory task with prepared fixtures. Required host and Windows
lints, all-target typechecking, docs, formatting and managed drift checks must
pass. Local macOS success does not establish ARM execution; corrected remote CI
remains required. The existing official failures are before-fix evidence; no
duplicate baseline run is needed.

## Surfaces

- [ ] interactive — no interactive surface changes
- [ ] deploy — no execution topology changes
- [ ] integration — no external contract changes
- [ ] agent-behavior — no agent behavior changes
