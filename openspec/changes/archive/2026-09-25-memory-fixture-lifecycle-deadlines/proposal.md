## Why

The archived `facade-fixture-deadlines` change derived facade fixture backstops from the product budgets. Other `kuru-memory` fixtures still wrap real Dolt service and store lifecycles in flat 90–120s outer timeouts, so they can fail the same way under a valid cold runtime-cache wait or a slow native lifecycle. The facade helper also left out two costs: an explicit owner close's reap budget, and idle retirement.

## What Changes

- `packages/kuru-memory/src/test_support.rs`: a shared test-only `fixture_deadline(n)`. It allows one `provision::LOCK_TIMEOUT` for a cold shared runtime install. Each real lifecycle then adds `startup_timeout_secs` for its open, `QUERY_TIMEOUT` for operations, and the larger of two retirement bounds. The first is the maintenance permit's startup deadline, which covers the owner's reap. The second is the server's own close budget from `server.rs`: two `CLOSE_GRACE` pool drains, `KILL_GRACE` and the supervisor reap allowance. Fixtures whose owner retires only through idle expiry add `SERVICE_IDLE_TIMEOUT` for each such lifecycle.
- `packages/kuru-memory/src/server.rs`: a test-only `close_budget()` mirroring `close_pools_and_owner` and `finish_owner`. Product constants and behavior do not change.
- `packages/kuru-memory/src/facade.rs`: the 17 facade sites use the shared helper, and their lifecycle counts are unchanged.
- `packages/kuru-memory/src/service.rs`: replace the flat outer backstops around real owner lifecycles, including the two Windows-only fixtures and the Unix-only cold-starter fixture.
- `packages/kuru-memory/src/store.rs`: replace the flat outer backstops around real service and store lifecycles.
- Inner short waits and all assertions stay unchanged. Inner waits that bound a cold start (starter readiness, cold-starter output, Windows ready-marker observations) are reported, not changed.

## Impact

Test-only behavior. Facade bounds grow by 2s per lifecycle because the explicit-close budget (32s) is now the larger retirement bound. CI time for passing runs is unchanged. The Windows coverage prefetch stays a separate CI change.
