## Why

Since #87, native Windows coverage has hung in `authorized_web_fetch_overlaps_checked_search_and_keeps_result_order` until the 90-minute job limit. Two fixture defects combine to cause it:

- The test's HTTP fixture server starts its ten-second bound when the spawned server task first runs, during the first yield inside `MemoryStore::temporary()`. Cold managed-memory startup, harness construction and turn admission therefore all count against a bound meant for one released web request. When those run slowly, the server panics and drops its request signal.
- The test then fails its bounded request wait. At that point the checked `grep` read is still parked in `ParallelReadTestGate` inside `spawn_blocking`. Dropping the Tokio runtime waits for blocking workers with no bound, so the panic turns into a silent hang instead of a FAILED result.

#87 (`d38e1a9b`) changed code on that setup path, which makes it a likely contributor to slower Windows setup: it changed `packages/kuru-memory/src/store/migrations.rs` (+748/−74), which `MemoryStore::temporary()` runs, and `packages/kuru-runtime/src/engine.rs` (+1602/−157) for session catalog and public transcript persistence, which `Harness::with_tool_host` and `run_controlled` perform before the tools run. The Windows `kuru_memory` suite also grew from about 16 to 23 minutes after #87. Cold-open and admission time on Windows has not been measured directly.

## What Changes

- In `packages/kuru-runtime/src/tests.rs`, move the existing release-on-drop guard to module level as `ReleaseGateOnDrop`. Hold it in every `ParallelReadTestGate` fixture:
  - `authorized_native_reads_overlap_but_feed_results_back_in_provider_order`
  - `authorized_web_fetch_overlaps_checked_search_and_keeps_result_order`
  - `cancelled_parallel_wave_drains_every_owned_read_before_returning`
  - `refused_parallel_read_does_not_replay_a_later_accepted_serial_effect`, which already had the guard

  A failed bounded wait now reports its step instead of hanging during runtime shutdown.
- In the web fetch overlap fixture, stop timing the server from when it is first polled. Its two existing ten-second bounds now start at the transport steps they observe: reading the request headers after accept, and writing the response after the test releases it. The listener's accept remains bounded by the test's own request wait, which starts after the web call is released. That request wait now names the step if the server drops its request signal.
- Each gated fixture's wait for its turn to reach checked execution previously used a fixed ten-second literal. It now covers #87's durable admission writes, so it uses the temporary store's configured memory startup budget, `MemoryConfig::default().startup_timeout_secs`. The store's Dolt listener derives its statement read timeout from at least that budget. The four gated fixtures use the same derived wait.
- Every other wait value, fixture step and assertion is unchanged.

## Impact

This changes test code in `packages/kuru-runtime/src/tests.rs` only. Product behavior, runtime and memory budgets, workflows, and coverage scope are unchanged. Native Windows CI for this exact head has not run yet and is required before merge.
