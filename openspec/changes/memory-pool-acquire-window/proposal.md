# Proposal

## Why

Every memory pool built by `Server::pool` gives SQLx a 2 s lifetime
`acquire_timeout` (`ORDINARY_POOL_WINDOW`, `packages/kuru-memory/src/server.rs:57`,
applied through `pool_attempt()` :998-1003 and `.acquire_timeout(..)` :2244).
SQLx starts that timer at the acquire's first poll and charges permit waits,
idle-connection pings, TCP and MySQL authentication and Kuru's identity
callback to it (SQLx 0.9.0 `sqlx-core/src/pool/inner.rs:246-297`). The 2 s is
a guess about runner speed, not a product budget: the statement the acquire
serves already runs under `QUERY_TIMEOUT` (30 s, `store.rs:66`), so a loaded
runner fails correct work with `PoolTimedOut` long before the work's own budget
ends. #170 (`memory-pool-acquire-at-cause`) removed the connection churn that
made new connections frequent and named what a timed-out acquire waited for,
but left the window itself in place; the family still occurs (Windows coverage
run 36930175874 job 110597536882, PR jobs 110030056329 and 110151458124).

The 2 s bound also misreports its cause: when an enclosing statement budget
and the pool window disagree, the error chain does not say which bound ended
the wait, and a budget that expires during statement execution is
indistinguishable from an acquisition timeout. A post-open pool creation
additionally retries an authored identity rejection through SQLx's connect
loop until the window ends instead of reporting it at once
(`identity_rejection_is_terminal: opening`, server.rs:1007).

## What Changes

- Once memory is open, a pool acquisition is bounded by the remaining budget of
  the statement or operation it serves, through a budget scope
  (`pool::within` / `pool::within_until`) that replaces
  `tokio::time::timeout` / `timeout_at` at memory statement sites. The pool's
  SQLx lifetime ceiling becomes `QUERY_TIMEOUT`, so no acquire bound exceeds a
  statement budget; nothing is set above an existing budget (lead decision
  D-2 (a)): the diagnostic names whichever of the scope or the ceiling fired.
- A receipt-bearing write takes one deadline before its acquire that covers the
  acquire, the identity statement, any pre-`Pending` validation and the apply
  (lead decision D-1 (a)), so every service write completes inside the
  client's 35 s `OPERATION_TIMEOUT` by construction. The uncertain-write fence
  is unchanged: `Pending` is still set only after the acquire and identity
  statement.
- A post-open pool creation (first acquire, first release and identity
  verification) runs under one creation budget (lead decision D-3 (b)). The
  opening phase keeps its startup-derived first acquire, its 2 s floor and its
  retry branch; `startup_timeout_secs` keeps its value and meaning.
- `PoolAcquireTimedOut` names its bound (statement budget or pool ceiling), the
  budget and the acquire's share; a budget that expires during execution is a
  `BudgetElapsed` with no acquisition diagnostic.
- A pending acquisition past a 2 s slow-acquire threshold leaves a diagnostics
  record; the threshold only logs and never decides an outcome.
- An authored identity rejection is terminal on every post-open path and
  sticky for the pool's life.
- Values unchanged: `startup_timeout_secs`, `QUERY_TIMEOUT`,
  `OPERATION_TIMEOUT`, pool `max`, Dolt listener settings, the attach probe and
  the service outcome probe.

Stated losses: the opening first-acquire retry branch (server.rs:2329) stays
for configured startup budgets above `QUERY_TIMEOUT`, so it is reachable only
with `startup_timeout_secs` over 30 and no behavior test reaches it; the
opening first-release cut loses its only behavior test under the single
post-open creation budget.

## Capabilities

### New Capabilities

### Modified Capabilities

- `versioned-memory`: "Pooled SQL session reuse" bounds a post-open pool's
  first connection by its creation budget and allows the acquire bound to
  change only as the acquisition budget requirement states; "Diagnosed memory
  pool acquisition timeout" bounds acquisitions by the served work's budget,
  names the deciding bound, separates execution expiry from acquisition
  timeouts, keeps acquisition failures before a write certain, makes identity
  rejections terminal and sticky, and records slow acquisitions.

## Impact

- `packages/kuru-memory/src/pool.rs`: budget scope, in-flight slot, pending
  count, slow-acquire records, typed bound fields, `BudgetElapsed`, sticky
  identity rejection.
- `packages/kuru-memory/src/server.rs`: pool ceiling, post-open creation
  budget, terminal identity rejection, constant split, test seams.
- `packages/kuru-memory/src/store.rs` and submodules: memory statement budgets
  converted to the budget scope; receipt-bearing writers take one write
  deadline before their acquire.
- `packages/kuru-memory/src/service/rpc.rs`: `PROBE_BUDGET` becomes its own
  constant; the `REPLY_MARGIN` comment states the one-write-budget property.
- `docs/development.md`: "Memory pool acquire timeouts".
- No configuration, on-disk format, wire format or public API change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
