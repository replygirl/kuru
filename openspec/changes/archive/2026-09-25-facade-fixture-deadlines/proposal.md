## Why

Windows coverage run 36210125264 (job 108314719071) failed `facade::tests::candidate_begin_recovery_keeps_clones_fenced_until_exact_ref_reattaches` on its flat 90-second outer fixture timeout. The Windows coverage shard does not prefetch the Dolt runtime, and `kuru_memory`'s lib tests run first in that shard, so the first two fixtures started with a cold `kuru-dolt-test-cache` and competed for one installation. The provisioner allows that wait up to `LOCK_TIMEOUT`. On that runner both first-batch fixtures shifted by about 60–70 seconds: `cancelling_after_accepted_unit_frame…` took 77s, compared with 9–16s in the three sibling jobs. Fixtures that started after the cache was warm ran at their usual speed (`candidate_promotion…` took 10.7s, compared with 6–9s). `candidate_begin…` (three real service lifecycles, 17–28s in the sibling jobs) then crossed 90s. The recovery path itself was not slow; the flat bound was smaller than the product budgets the fixture legitimately exercises.

## What Changes

- `packages/kuru-memory/src/facade.rs` tests: replace all 17 flat outer fixture timeouts (fifteen 90s bounds, one 60s bound and one 150s bound) with a deadline derived from the configured budgets. One `provision::LOCK_TIMEOUT` covers waiting behind a cold shared runtime install. Then each real service lifecycle (every `ServiceOwner::open` or local `MemoryStore::open` that starts Dolt) adds `startup_timeout_secs` for its store open, `QUERY_TIMEOUT` for the fixture's operations, and `startup_timeout_secs` for `acquire_maintenance_permit`'s election and retirement deadline, which covers the owner's Dolt reap. Each site names its lifecycle count, and its failure context reports the derived bound. This mirrors `recovery_tests.rs`'s `migration_observation_deadline`.
- The deadline stays a hang backstop above the product's own inner budgets, so a real startup, lock, query or retirement stall reports its specific product error first.
- Keep every inner short wait: pause notifications, witness polls, writer cancellation, reap-after-permit and fast refusals. Keep every assertion.
- `packages/kuru-memory/src/provision.rs`: `LOCK_TIMEOUT` becomes `pub(crate)` so the fixtures can name it. Its value and behavior do not change.

## Impact

Test-only behavior. Facade fixtures still bound hangs, but no longer fail when a valid cold-cache wait or slow native lifecycle stays within the configured product budgets. Passing-run CI time is unchanged. Out of scope: the Windows coverage shard does not run the memory `prefetch` step that the package `test` task depends on. That is the wall-clock fix for the cold first batch and belongs in a separate delivery/CI change.
