## Why

The exact post-merge macOS Intel memory check resumed cold migration recovery, then its test-only 10-second outer wait expired before the configured startup and query budget. Other full cold opens in the same recovery fixture use that same too-short wait.

## What Changes

- Use the existing configured migration observation deadline for full cold open and recovery awaits in `packages/kuru-memory/src/store/recovery_tests.rs`.
- Use the same deadline for each first-boundary wait whose observed boundary follows a fresh `spawn_gated_open` Dolt start: the routed-source waits in the lost commit, branch, fast-forward and absent-publish reply fixtures; the accepted-cancellation `control.reached()` immediately after spawning, which reuses its already-computed `completion_deadline`; and the process-loss contender's takeover after the creator is reaped, which must take the startup guard and start Dolt before reaching its paused boundary.
- Keep second-stage `control.reached()` waits after `resume_route()` on the short deadline: Dolt is already running when they start.
- Keep short fault, cleanup and refusal deadlines and all recovery assertions intact.

## Impact

Only the recovery test fixture changes. The product startup/query deadlines and CI job configuration stay as they are; slow but valid native recovery gets the configured observation window.

## Considered and left unchanged

Two post-resume waits in this file keep the short deadline even though they
look like the extended ones:

- `recovery_tests.rs` absent fast-forward refusal (`opening` after
  `control.resume()`, "absent fast-forward did not resolve"): Dolt is already
  running when the wait starts. The successful fast-forward wait was extended
  because, after its migration worker closes, the open reopens the migrated
  store with a second Dolt start and readiness checks. The refusal returns the
  migration error once the worker's owned server has closed and starts no
  server, so its remaining work is owner shutdown, the lease-release class
  below, not a cold start.
- The usage-upgrade publication fixture (`route_source()` "usage upgrade did
  not expose publication route" and the settle wait "usage upgrade did not
  settle its publication attempt"): `upgrade_usage_with_hooks` runs against an
  already-open `released_server`, and the fixture never calls
  `spawn_gated_open`, so neither wait includes a server start.

Two other bare-10s waits were checked against this change's own discriminator
(does the wait observe a cold open/recovery bounded by `startup_timeout_secs`,
or a post-open lease-release/close/quiescence bounded by the owner-shutdown
budget?) and left alone because they are the latter, not the former:

- `packages/kuru-memory/src/store/recovery_tests.rs:311` (`take_quiescence`,
  `Server::quiescence_at(..., TEST_DEADLINE)`): waits for the *owner to
  release its lifecycle lease* after a close, not for a cold open. Its correct
  budget is the private owner-shutdown allowance (`CLOSE_GRACE + KILL_GRACE +
  2s` in `server.rs`), not `migration_observation_deadline`. No product
  constant is exported for that budget today, and this test is not observed
  failing, so widening it here would be speculative. Leave as `TEST_DEADLINE`;
  revisit as its own change if it is ever observed to flake.
- `packages/kuru-memory/src/store/migration_lifecycle_tests.rs:48`
  (`DEADLINE`, used at lines 110/130/176/212 of
  `interrupted_migration_close_handoff_retains_guard_until_supervisor_quiesces`):
  every one of these wraps a post-open step (pool-close entry, contender lock
  polling, reaper guard release, final `quiescence_at`) of a store the test
  already opened at lines 88-106 using the server's own
  `startup_timeout_secs` directly (no test-level deadline there at all). None
  of the four `DEADLINE` sites is a cold open or recovery wait, so this file
  is out of scope for this change; it is the same lease-release/shutdown
  class as the item above, not touched by this fix.
