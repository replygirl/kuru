# Proposal

## Why

Main CI run `37190628278`, Ubuntu coverage partition 2, failed launching the open-time retirement fixture with Linux `ETXTBSY` at `tests/open_time.rs:202`. Copying its executable in the multithreaded test parent lets a sibling's fork inherit the writable descriptor until exec, even after the parent's copy returns.

## What Changes

- In `packages/kuru-delivery/tests/open_time.rs`, copy the fixture with an awaited Unix child before launching it; the test parent never opens the destination for writing.
- Keep the existing retirement, census, and endpoint assertions; verify the test and full open-time integration binary.

## Impact

Test preparation adds one short Unix child invocation. Product behavior, open-time measurement, performance gates, and test wait budgets remain unchanged. Linux CI proves the affected platform; local macOS checks verify the integration behavior but cannot reproduce Linux's refusal.
