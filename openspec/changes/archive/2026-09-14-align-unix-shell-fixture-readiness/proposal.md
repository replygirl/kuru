## Why

`diagnostic_write_failure_keeps_a_completed_cli_turn_authoritative` failed in the pre-push suite after its fake-provider readiness poll used the five-second cleanup bound, even though the owned CLI operation has a 45-second fixture budget. The absence of a provider-stage observation within five seconds does not establish a production launch defect; the exact cold-start delay remains unproven.

## What Changes

- In `apps/kuru-tui/tests/unix_shell_turn.rs`, align this test's fake-provider readiness observation with the existing bounded CLI operation budget while retaining the shorter cleanup deadline for process and server cleanup.
- Add or adapt a deterministic held-startup regression so the readiness poll is proven to survive beyond the cleanup window and still preserve the existing diagnostic-replacement assertions.

## Impact

Only the Unix shell-turn fixture and its focused test evidence change. Production timeouts, provider behavior, cleanup bounds, subprocess ownership, output assertions, and normal successful runtime are unchanged; a failed readiness observation may now consume the fixture's existing CLI budget.
