# Design

## Context

Root cause [measured at origin/main 1752e076]: `ORDINARY_POOL_WINDOW` (2 s,
`packages/kuru-memory/src/server.rs:57`) is every `Server::pool` pool's SQLx
lifetime `acquire_timeout` (`pool_attempt()` :998-1003, `.acquire_timeout(..)`
:2244). SQLx 0.9.0 `PoolInner::acquire` (`sqlx-core/src/pool/inner.rs:246-297`)
starts that timer at the acquire's first poll and covers the permit wait, the
idle-connection ping and `connect` (TCP, MySQL authentication and Kuru's
`after_connect` identity callback). The statement each acquire serves already
runs under `QUERY_TIMEOUT` (30 s, `store.rs:66`), so the 2 s cuts correct work
short on a loaded runner and reports the wrong bound. The approved design,
with its critique and revision, is
`tmp/roadmap/d2-pool-acquire-window-design-2026-10-02.md` (untracked shared
notes); this artifact records the decisions it implements.

## Goals / Non-Goals

**Goals:**
- Bound every post-open acquisition by the remaining budget of the work it
  serves, with a pool ceiling equal to the statement budget.
- Name the deciding bound, the budget and the acquire's share in the typed
  diagnostic; keep execution expiry distinct from acquisition timeouts.
- Keep every service write inside the client's 35 s `OPERATION_TIMEOUT` by
  construction.
- Leave a diagnostics record for a slow pending acquisition.
- End an authored identity rejection at once on every post-open path.

**Non-Goals:**
- Any change to `startup_timeout_secs` (value or meaning), `QUERY_TIMEOUT`,
  `OPERATION_TIMEOUT`, pool `max`, Dolt listener values, the attach probe or
  the service outcome probe.
- Converting service IPC frame timeouts, the outcome probe, the guarded
  outcome reads in `service/rpc.rs`, `DREAM_LEASE_WAIT` lease waits,
  `CLOSE_GRACE` closes or process/supervisor timeouts.
- The opening-phase attach probe's 2 s (a separate unit).

## Decisions

- **Budget scope, not a per-site parameter.** `pool::within(budget, fut)` and
  `pool::within_until(deadline, fut)` replace `tokio::time::timeout` /
  `timeout_at` at memory statement sites. The scope is task-local; its
  deadline is the minimum of its own and any enclosing scope's. The pool funnel
  registers its in-flight acquire in the scope's slot, so an expiry during the
  acquire builds `PoolAcquireTimedOut { bound: StatementBudget }`. Rejected:
  threading an explicit budget value to every site (largest diff) and the bare
  pool option change (an acquire that exhausts the caller's budget is reported
  as an untyped deadline).
- **D-1 (a), lead decision: one write budget.** A receipt-bearing writer takes
  `deadline = now + QUERY_TIMEOUT` before its acquire and runs acquire, the
  `CONNECTION_ID()` identity statement, any pre-`Pending` validation and apply
  under `within_until(deadline, ..)`; the receipted release keeps that
  deadline. Every service write therefore completes inside the client's 35 s
  `OPERATION_TIMEOUT` (`QUERY_TIMEOUT` + `REPLY_MARGIN`, `service/rpc.rs:38`,
  :699-702) by construction. `Pending` is still set after the acquire and
  identity statement and before apply. Rejected: separate budgets, which let a
  contended write run past the client's 35 s.
- **D-2 (a), lead decision: the bound names whichever timer fired.** The scope
  and SQLx's lifetime ceiling (`QUERY_TIMEOUT`) are the only deciding timers;
  when their deadlines coincide on one tick, `bound` names the one that fired.
  Nothing is set above an existing budget. Rejected: a SQLx backstop above the
  budget with the funnel as the only deciding timer.
- **D-3 (b), lead decision: one creation budget.** A post-open
  `Server::pool()` runs its first acquire, first release and identity
  verification inside one `within(QUERY_TIMEOUT, ..)`, nested in any
  enclosing scope. A first release that outlasts it fails creation with no
  pool retained; nothing is retried. The opening phase keeps its
  startup-derived first window, 2 s floor, release cut and retry branch.
  Rejected: per-step nested budgets (unscoped creation up to 2 x 30 s).
- **Slow-acquire threshold.** `SLOW_ACQUIRE_THRESHOLD` (2 s) only logs: a
  pending acquire emits `memory pool acquire still waiting` once, and one that
  completes past it emits `memory pool acquire completed slowly`.
- **Sticky identity rejection.** `pool_attempt()` makes an authored identity
  rejection terminal always; a retained pool keeps one sticky rejection that
  the funnel selects on.

## Risks / Trade-offs

- [A permit stall now surfaces after up to 30 s, not 2 s] → the slow-acquire
  record at 2 s keeps it visible while pending.
- [SQLx's connect loop retries non-identity `after_connect` errors for up to
  the whole budget, with backoff capped at `remaining / 5` (inner.rs:333)] →
  SQLx's existing retry, not a new one; the error still names its phase at the
  budget.
- [D-1 (a): a contended acquire consumes apply time, so more writes may end
  uncertain under heavy contention] → reconciled by the unchanged
  `resolve_uncertain` fence.
- [Task-local scope lost across `tokio::spawn` or a stream polled after its
  scope] → falls back to the 30 s ceiling, typed as `PoolCeiling`.
- [Coverage] → the opening retry branch (server.rs:2329) is reachable only
  with `startup_timeout_secs` over 30, and the opening first-release cut loses
  its only behavior test under D-3 (b); both line-coverage deltas are reported
  against the 90 % gate.
