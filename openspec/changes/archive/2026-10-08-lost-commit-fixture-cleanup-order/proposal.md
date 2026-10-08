# Proposal

## Why

The lost-commit fixture's early-error cleanup closes the engine before its packet
proxy finishes observing SQL session teardown. Ubuntu coverage partition 7 in
PR #263 exposed the resulting proxy cleanup failure.

## What Changes

- In `packages/kuru-memory/src/store/recovery_tests.rs`, retain the returned
  opening store, await the existing session-end observation, retire the proxy,
  then close the store.
- Preserve the original error, quiescence assertions and existing deadlines.

## Impact

Test fixture cleanup only. Production behavior, workflows and mise files remain
unchanged. Run both existing production lost-commit acceptance cases and the
normal static checks; full PR and exact-main native coverage remain required.
