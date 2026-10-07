# Proposal

## Why

The real stand-in owner's progress-wait fixture infers consumption from a removed release filename. Windows x64 CI failed that negative namespace probe with access denied after all readiness behavior had run; disappearance is an unsuitable acknowledgement and the observed log does not identify the exact pending handle.

## What Changes

The test-support stand-in atomically renames its fully read release to a sibling consumed receipt immediately before exit. The fixture observes and validates that positive receipt against the first released status, rejecting actual I/O, parse and status mismatches without changing its existing real-time bound or poll interval. Keep the original progress, stale/lower-count, status-three, lifecycle and readiness assertions, with explicit receipt regressions.

## Capabilities

### New Capabilities

### Modified Capabilities

None; existing production lifecycle and readiness contracts remain correct.

## Impact

Only the test-support stand-in and progress-wait fixture in `packages/kuru-memory/src/service.rs`, coordinated owning prose in `docs/development.md`, plus this typed fix record. No production ownership, platform ACL, timeout, provider, configuration, dependency or protocol change. A receipt proves consumption immediately before exit, not completed reap. Native Windows correction remains pending fresh full CI.

## Surfaces

- [ ] interactive — no user interface change
- [ ] deploy — no execution topology change
- [ ] integration — no external contract change
- [ ] agent-behavior — no inference behavior change
