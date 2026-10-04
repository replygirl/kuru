# Proposal

## Why

PR #179 replaced two flat waits with `WRAPPER_LAUNCH_BUDGET` (180 s), tied only by a pin test to three sibling 180 s constants that have no written derivation of their own. No literal may stand without a derivation, and a wait ends on the event it waits for under a stated budget. Separately, the open-time documentation still says exit is stamped "as it is read", while the settled behavior (#133) stamps `exit_ms` on Unix, as on Windows, only after both pipes close and the command is reaped.

## What Changes

- `packages/kuru-delivery/tests/bootstrap_windows.rs` (`TIMEOUT`), `tests/support/mise_acceptance.rs` (`DEADLINE`), `tests/support/previous_updater.rs` (`DEADLINE`) and `tests/powershell_diagnostics.rs` (`WRAPPER_LAUNCH_BUDGET`): each 180 s constant is either given a written derivation from a product or vendor budget it actually depends on, or replaced by the event it should wait for under an existing budget. No new literal without a derivation, no raised literal, no retry, and no change to what any test asserts.
- The derivations are recorded once, beside the constants (a derivation record the others cite), not repeated per file. Only a budget that already exists in product code may be exposed to test support; no other product code changes.
- The `powershell_diagnostics.rs` pin test (`wrapper_waits_take_the_package_launch_budget_inside_the_shard_deadline`) is updated to pin the derivation instead of numeric equality with the siblings, while keeping its call-site counts and its coverage shard deadline check.
- `docs/development.md` open-time section: the line saying exit is stamped "as it is read" states that exit is stamped after both pipes close and the command is reaped.

## Impact

Test support and test constants in `packages/kuru-delivery` and one documentation line. No product behavior, workflow or dependency changes; coverage and CI time are unchanged unless a replaced wait now ends sooner on its event. `docs:check` must pass.
