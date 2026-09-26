## Why

An independent review of `facade-fixture-deadlines` and `memory-fixture-lifecycle-deadlines` found four problems:

- Each fixture's deadline charged `LOCK_TIMEOUT` for a shared cold runtime install. That wait happens once per test process, not once per fixture.
- A flat per-lifecycle term undercounted a fresh store open. A fresh open starts and closes Dolt several times, so the deadline did not always expire after the product's own error.
- The supervisor reap allowance was an unnamed literal that the test model had copied by hand.
- Two inner readiness waits were still tied to the outer fixture bound.

Both archived records also named the wrong follow-up. `AGENTS.md` says instrumented fixtures provision their own engine cache, so the answer is not running coverage `prefetch`. It is the in-binary warm-up added here.

## What Changes

- **Runtime warm-up** (`packages/kuru-memory/src/test_support.rs`): a `pub` `warm_runtime_cache()`, compiled for unit tests and the `test-support` feature. It provisions the bundled runtime into `test_cache()` once per test process, bounded by the product's cache-lock wait `LOCK_TIMEOUT`, and caches its result. Every budget-bounded fixture calls it before starting its outer timeout. The Windows lifecycle, supervisor snapshot and server lifecycle integration tests use it instead of their own unbounded `provision` calls.
- **Fresh versus reopen model** (`test_support.rs`): `fixture_deadline(fresh, reopened)` replaces the flat term. Each budget below is one product timeout or grace:
  - A **fresh store open** allows the startup lock wait; four server starts, each `startup_timeout_secs` plus `SUPERVISOR_TRANSPORT_ALLOWANCE`; three owned server closes (`close_budget`); the stage quiescence wait (`startup_timeout_secs`); and one `QUERY_TIMEOUT` per server session.
  - A **reopen** allows the lock wait, one server start and one `QUERY_TIMEOUT`.
  - **Every lifecycle** also adds one `QUERY_TIMEOUT` for the fixture's own operations, and a retirement bound: the larger of the maintenance permit's startup deadline and one owned close.
  - Owners that retire only through idle expiry still add `SERVICE_IDLE_TIMEOUT` at the call site.
  - The doc comment claims only what the model supports: a single stalled product step reports its own error before the fixture bound expires.
- **Named server allowances** (`packages/kuru-memory/src/server.rs`): `SUPERVISOR_TRANSPORT_ALLOWANCE` (2s) and `SUPERVISOR_REAP_ALLOWANCE` (`CLOSE_GRACE + KILL_GRACE +` transport allowance) become named constants. `Server::open`'s startup deadline, `finish_owner` and the test-only `close_budget()` use them. A dropped owner's warning threshold is the reap allowance plus a named 1s margin. All values stay the same.
- **Inner readiness waits** (`packages/kuru-memory/src/service.rs`):
  - The Windows starter readiness waits (denying outer job, and starter job exit) now allow the product's `attach_or_start` startup cap, one handshake, the starter's append call and a named child-start margin. Each is capped just below the outer fixture deadline.
  - The crashed-owner readiness wait allows one fresh open plus the child-start margin, with the same cap.
- **Call sites**: every `fixture_deadline` site in `facade.rs`, `service.rs` and `store.rs` passes its counted fresh and reopened lifecycles. Other inner waits and all assertions are unchanged.

## Impact

Test fixtures and test helpers change. In product code, only named constants replace literals, with identical values. Per-fixture backstops grow to match the fresh-open model: 466s for one fresh lifecycle and 154s for each reopen. Wall time for passing runs is unchanged, apart from one warm-up per test binary, which does the same cold install a first fixture would have done.
