## Why

The Windows lost-commit-reply migration fixture reached its injection point but its 10-second outer watchdog expired before the existing accepted migration and server reopen bounds. The test should allow the established migration observation window and identify which proxy step completed if that window expires.

## What Changes

- In `packages/kuru-memory/src/store/recovery_tests.rs`, use the existing migration observation deadline for this one opening await and report the already tracked discarded-reply and ended-session flags on timeout.

## Impact

Test-only change. No production timeout, migration behavior, or public API changes; this one failure path may run up to the existing 60-second fixture observation window.
