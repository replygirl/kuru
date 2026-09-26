## 1. Product allowances

- [x] 1.1 Name `SUPERVISOR_TRANSPORT_ALLOWANCE` and `SUPERVISOR_REAP_ALLOWANCE` in `packages/kuru-memory/src/server.rs`, use them in `Server::open`, `finish_owner`, the dropped-owner warning and the test-only `close_budget()` without changing any value, and verify by review and the server tests.

## 2. Test runtime warm-up

- [x] 2.1 Add `test_support::warm_runtime_cache()` (once per process, bounded by `LOCK_TIMEOUT`, spawn-gated under unit tests), call it before every budget-bounded fixture's outer timeout, and use it in the Windows lifecycle, supervisor snapshot and server lifecycle integration tests; verify the warm-up runs before the fixtures in a cold-cache run.

## 3. Fresh and reopen budget model

- [x] 3.1 Replace the flat lifecycle term with `fixture_deadline(fresh, reopened)` built from the fresh-open and reopen models, recount every call site in `facade.rs`, `service.rs` and `store.rs`, and verify the counts against each fixture body.
- [x] 3.2 Give the Windows starter and crashed-owner readiness waits their own derived bounds, capped below the fixture deadline, and verify the crashed-owner fixture passes.

## 4. Evidence

- [x] 4.1 Run the affected facade, service, store and integration tests through the package mise test task, run a cold-cache first batch, run lint, and record the results, naming the Windows-only checks that could not run here.

## Budget model (default configuration)

- **Base values:** startup 30s; server start 32s (30s plus the 2s `SUPERVISOR_TRANSPORT_ALLOWANCE`); `QUERY_TIMEOUT` 30s. `close_budget()` is 32s: an 8s drain, 3s `KILL_GRACE`, 13s `SUPERVISOR_REAP_ALLOWANCE` and a second 8s drain.
- **Fresh open:** 30s lock + 4 × (32s + 30s) + 3 × 32s + 30s quiescence = 404s.
- **Reopen:** 30s lock + 32s + 30s = 92s.
- **Settle, per lifecycle:** 30s for fixture operations + max(30s, 32s) retirement = 62s.
- **Resulting deadlines:** fixtures with one fresh lifecycle get 466s, and each reopen adds 154s. Common totals are (1,0) 466s, (1,1) 620s, (1,3) 928s, (2,1) 1086s and (2,4) 1548s. Idle-retiring owners add 30s.
- **Inner readiness waits:**
  - Windows starter readiness: 30s startup + 3 × 5s handshake + 30s append + 5s `CHILD_START_MARGIN` = 80s. It was 40s in one fixture and "the fixture deadline minus 2s" in the other.
  - Crashed-owner readiness: 404s + 5s = 409s. It was "the fixture deadline minus 2s".
  - Both are capped at the fixture deadline minus 2s.

## Recounted sites (fresh, reopened)

- **`facade.rs`:**
  - (1,1): public transcript, legacy continuation.
  - (2,4): selected abandonment.
  - (2,1): candidate unit, candidate begin.
  - (1,0): all 12 others.
- **`service.rs`:**
  - (1,0): maintenance, retiring handshake, owner reaps, idle accept.
  - (1,0) + idle: starter job exit (Windows), elected process, cold starters (Unix).
  - (1,1): denying outer job (Windows), lost reply restart, crashed owner, lost usage, transition query, lost transitions, rejected publication.
  - (1,3): lost Begin reply.
- **`store.rs`:**
  - (1,3): service disconnect and restart.
  - (1,0): cleanup retry, transition observation, selected inventory.
  - (1,1): exact outcome across restart.

## Corrections to archived records

The earlier records `2026-09-25-facade-fixture-deadlines` and `2026-09-25-memory-fixture-lifecycle-deadlines` named the follow-up as running coverage `prefetch` in the Windows coverage shard. That is not right: `AGENTS.md` says instrumented fixtures provision their own engine cache, with no ordinary supervisor build first. The fix for the cold first batch is this change's in-binary `warm_runtime_cache()`. The per-fixture `LOCK_TIMEOUT` term those records describe has been removed. The earlier record's inner-wait list is superseded: both Windows starter readiness waits and the crashed-owner readiness wait now have their own derived bounds. The other flagged inner waits are unchanged, and the warm-up keeps a cold install out of them: Unix cold-starter output, and the Windows ready-marker observations, whose tests now obtain the engine from the warm-up.

## Findings for follow-up (not changed)

- **`attach_or_start` readiness cap** (`service.rs`): readiness is capped at `startup_timeout_secs` (30s) in total. A spawned owner's fresh store open is bounded above by four server starts and three closes. So a first launch on a slow host can fail with "memory service readiness deadline exceeded" while its owner is still initializing within its own budgets.
- **`MemoryStore::temporary()`**: it waits on an in-process 4-permit semaphore with no bound, and that wait sits inside the fixture deadline.
- **`spawn_gate`**: waits inside fixture deadlines still have no product budget, as reported earlier.

## Observed evidence

Local macOS aarch64 run, 2026-09-25:

- `mise run //packages/kuru-memory:test -- --test server_lifecycle`: the mise task keeps `--all-targets`, so the whole `kuru-memory` suite ran. The lib tests passed 267 of 267 (535.43s). The integration tests all passed: `memory.rs` 5, `server_lifecycle` 12, `supervisor_snapshot` 1 and `bundle_build` 6.
- Focused `service::tests`, `facade::tests`, the five changed store fixtures and the snapshot test: 67 passed plus 1 passed.
- `test_support::tests::runtime_warm_up_is_shared_and_returns_the_cached_engine` passed with the prefetched cache. It also passed against an empty `KURU_DOLT_CACHE` in 3.87s, including the cold install.
- Cold first batch (empty `KURU_DOLT_CACHE`, 2 threads, catalog plus `cancelling…`, `candidate_begin…` and `candidate_promotion…`): all passed. Without throttling the batch took 14.93s; under `taskpolicy -b` it took 29.02s. The host load was lower than in the earlier runs.
- `mise run //packages/kuru-memory:lint` (clippy `-D warnings`) and `cargo fmt --check` are clean.
- Not run here:
  - The Windows-only code (the two `#[cfg(windows)]` `service.rs` fixtures, `windows_starter_readiness()` and `tests/windows_lifecycle.rs`) could not be built. `cargo check --target x86_64-pc-windows-msvc` fails in `libsqlite3-sys` without the Windows C headers.
  - Native Windows CI on the exact commit is the gate.
