# Proposal

## Why

Rust 1.99 deprecates Atomic::fetch_update in favor of Atomic::try_update. The dependency refresh therefore fails the existing warnings-as-errors gate in tool admission and memory activity tracking.

## What Changes

Use the supported atomic method name at the existing call sites, preserving the same closure, memory orderings and return handling. Verify the existing admission and activity tests and host/Windows lint.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

packages/kuru-connectors/src/tools/parallel.rs and packages/kuru-memory/src/service/activity.rs, with the dependency change keeping compiler and minimum Rust requirements coherent. No runtime contract or specification changes.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
