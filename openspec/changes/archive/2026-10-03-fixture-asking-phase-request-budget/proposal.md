# Proposal

## Why

`test_support::retire_idle_service_traced` bounds its asking phase (the requests it makes until one finds the owner closing or yields a maintenance permit) by `memory.startup_timeout_secs`, a figure the fixture chose. The product's own maintenance request budget is `service::maintenance_deadline` (the longer of the owner's close budget and `startup_timeout_secs`, introduced with the owner-retire bound in #202). It is what every attempt enforces on its start-lock wait, owner response and owner-lock wait.

When `startup_timeout_secs` is below the close budget, the fixture therefore gives up on an owner after the shorter figure while the product would still be waiting within its own budget. That reports a failure for an owner whose close is still within what the product allows, and the expiry text names a setting that is not the bound the product applies.

## What Changes

- `service::maintenance_deadline` becomes `pub(crate)`, visible to test support.
- The asking phase waits under `maintenance_deadline(options)` from its first request. The closing-phase backstop (`server::close_budget` from the first closing reading) is unchanged.
- The asking-phase expiry names the phase and the budget: `managed owner retirement asking phase: no request found the owner closing within the maintenance request budget (<budget>; <N>ms since the first request); <owner state>; <trace>; active-client refusals=<R>`. It keeps the owner-state reading, the trace and the refusal count.
- The doc comment of `retire_idle_service` and the fixture-retirement text of `docs/development.md` describe the two bounds accordingly.
- Tests: `ensure_asking_deadline_expiry` becomes `ensure_asking_phase_expiry` and derives the expected budget from `maintenance_deadline(options)` rather than from a caller-supplied duration. The two activity tests that assert the asking expiry use it. A new paused-clock test sets `startup_timeout_secs = 1` with an owner that never answers and asserts the expiry comes at the close budget and names the asking phase.

No new constant. No change to product behavior: only the fixture's bound and diagnostic move.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-memory/src/test_support.rs`, a one-word visibility change in `packages/kuru-memory/src/service.rs`, tests in `packages/kuru-memory/src/service/activity.rs`, and the fixture-retirement text in `docs/development.md`. No public API, configuration or product runtime behavior changes.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
