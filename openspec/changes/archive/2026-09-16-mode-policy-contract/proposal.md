## Why

The four built-in frameworks currently share hardcoded runtime dispatch, facing, visibility and memory rules. Phase 1 needs a validated six-axis contract before those loops are extracted, so later mode work can use explicit decisions while the runtime keeps authority over execution, budgets and persistence.

P3 captures the existing behavior as a baseline first. Without that baseline, P8 and P9 cannot demonstrate that extracting mode decisions preserved the current roles, equal peers, requests and memory isolation.

## What Changes

- Define independently callable pure roles, peering, flow, facing, visibility and memory policy components in `kuru-core`, composed into validated profiles for the four existing `Mode` values.
- Move the current built-in seed definitions into reference profiles while preserving their serialized modes, names, roles, stable IDs, instruction bytes, authored order and role coverage.
- Capture pre-extraction deterministic core and runtime baseline scenarios for all four modes, including targeting, relationship speaking, activation, continuity, cold ties and dream fallback.
- Keep engine, actor and dream loops as their current executors; P8 and P9 will route through every policy axis in later changes. This change adds no user mode, focus control, provider or memory authority.

## Capabilities

### New Capabilities

- `mode-policy-contract`: validated, pure six-axis reference profiles and behavior-preserving pre-extraction baselines.

### Modified Capabilities

None. Existing peer-cognition behavior remains the same.

## Impact

- `packages/kuru-core/src/framework.rs` and a focused policy module expose a non-serialized decision API; `Mode`, `Part`, `Relationship`, and persistent topology formats remain unchanged.
- Scoped core and `packages/kuru-runtime` tests record the four built-ins' exact current decisions and representative scripted runtime outcomes before P8/P9 extraction.
- Architecture/framework documentation describes the contract and its current limits. No storage migration, connector API, CLI command, or new dependency is required.

## Surfaces

- [ ] interactive — no user-visible control or output change
- [ ] deploy — no deployment/runtime topology change
- [ ] integration — no external contract change
- [x] agent-behavior — policy reference definitions and request/selection baselines protect model-facing behavior
