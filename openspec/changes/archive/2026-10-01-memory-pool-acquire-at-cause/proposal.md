# Proposal

## Why

Three Windows coverage runs failed with the bare SQLx text `pool timed out while
waiting for an open connection`: main run 36930175874 job 110597536829
(partition 8, `mode_baseline_tests.rs:550:44`, the `.unwrap()` of a dream, as
`actor memory operation failed: ...`), PR #142 job 110030056329 (partition 3,
`accounting_tests.rs:264:49`, the warmed `MemoryStore::temporary()` open) and
PR #145 job 110151458124 (partition 5, `review_tests.rs:1497:59`, a cold
`MemoryStore::open`). The error has no context, so it came from an acquire on
an already-created pool propagated with a bare `?`; none of the three logs
says whether the wait was for a held connection, an idle-connection check or
a new connection's authentication.

The cause in our code is connection churn charged to the 2 s ordinary acquire
window (`ORDINARY_POOL_WINDOW`), which SQLx applies to every later acquire on a
pool:

- Every durable write detaches its pooled connection (`owned_connection`) and
  closes it after the write, whatever the outcome. The next statement on that
  pool must open, authenticate and run the `after_connect` identity checks on a
  new connection inside the 2 s window.
- Dropping a pooled connection spawns SQLx's release task, which pings before
  the connection is idle again. On the `current_thread` test runtime the same
  task's next statement reaches the idle queue before that task runs, finds
  nothing and opens at least one extra connection per pool, more when release
  pings lag (the loaded-runner case). This lands on young pools: every open
  sequence and every dream's fresh candidate pool.
- A timeout is undiagnosable: `PoolTimedOut` is a unit variant raised alike for
  a permit wait, an idle ping, TCP/MySQL authentication and a slow
  `after_connect`.

Evidence limits: nothing in the three logs discriminates the waits; the new
connection mechanism is inferred from code. The idle-connection ping and a
reconnect after Dolt's per-read timeout (`read_timeout_millis`, 30 s by
default; the pinned Dolt 2.3.5 vitess `ConnWithTimeouts.Read` re-arms that
deadline before every read, including the wait for the next command, so an
idle session older than 30 s is closed by the server) are unmeasured
candidates that this change does not remove.

## What Changes

- A `MemoryPool` funnel owns every store, usage-ledger, export, migration and
  stage pool handed out by `Server::pool`. It has no `Deref` to the SQLx pool,
  so every acquisition goes through it. Its `Executor` implementation returns
  each statement's connection to the pool inline (SQLx's own eager release)
  before the statement's future completes, so sequential work on a pool reuses
  one authenticated session instead of racing the spawned release.
  `Server::pool` releases the pool's first connection inline, bounded by the
  pool attempt's own deadline, and runs the identity verification on that same
  session; a release that outlasts the deadline is dropped, so SQLx closes the
  connection and verification opens its own under its own deadline.
- A write session returns to the pool only after a receipted success
  (`Ok(Ok(_))`, the branch that already clears the uncertain-write record) in
  `mutate`, `mutate_session_catalog` and the usage ledger's `change`. Every
  other outcome (SQL error, `QUERY_TIMEOUT` elapsed, typed conflict, injected
  post-commit failure, early validation error) closes the session before
  `resolve_uncertain` runs, byte-for-byte today's fence. Each covered `apply`
  ends with `COMMIT` (or an explicit `ROLLBACK` for an unchanged usage
  settlement); no store write sets session variables, `USE`s a database,
  checks out a Dolt branch or takes a named lock. The receipted return is
  bounded by the write's own `QUERY_TIMEOUT` budget, one deadline taken before
  its apply: a return that outlasts it is dropped, SQLx closes the floating
  connection and lowers the pool size, and the receipted write still returns
  `Ok`. Without that bound an awaited release ping that stalled would hold the
  write mutex, every later write and `MemoryStore::close`.
- Candidate creation, promotion, transition, deletion and exclusion writers
  keep closing their sessions on every outcome. They always reconcile through
  the processlist, run `DOLT_BRANCH`/`DOLT_MERGE` procedures and are not the
  writers whose churn this change targets.
- A pool acquire that SQLx times out reports a typed, secret-free
  `PoolAcquireTimedOut`: the branch, pool `max`, `size`, idle and checked-out
  counts, elapsed wait, window, connections authenticated since the pool opened
  and during this wait, the wait class, and the connection phase only when a
  connection entered Kuru's identity callback during this wait. The
  connection counter moves on the callback's first line, after TCP and MySQL
  authentication have finished. Wait classes: `held connections` (every
  permit held by Kuru work), `new connection` (a new connection reached the
  identity callback during the wait) and `no identity callback` (capacity
  existed and no new connection reached the callback: an idle-connection
  ping, a release in flight, or a TCP or MySQL handshake that did not finish,
  which Kuru cannot tell apart). The funnel's own `acquire`/`begin` return it as anyhow context over
  the original `sqlx::Error::PoolTimedOut`; statements run through the
  `Executor` carry it as `sqlx::Error::Io` of kind `Other` whose payload is the
  typed diagnostic, so the existing `MemoryFailure` and fixture panic chains
  print it unchanged. No classifier treats that carrier as a lost connection,
  an accept timeout or an uncertain write: the only `sqlx::Error::Io`
  classifier matches `ConnectionReset` during pool creation, and the service's
  timeout classifiers match `io::ErrorKind::TimedOut` only.
- The pool logs every timed-out acquisition as a `memory pool acquire timed
  out` warning with the same typed fields, so the memory service owner's log
  names the wait even though the wire fault stays `StorageFailed` (carrying
  the diagnostic to clients is decision D3, deferred).
- Test hooks: a per-pool authentication gate and release gate (no time
  involved; the release gate also has a take-once form that the next pool
  attempt can arm on its first connection) and an identity-callback
  connection counter retained per pool.

Not changed: `ORDINARY_POOL_WINDOW` (2 s), `startup_timeout_secs`,
`QUERY_TIMEOUT`, the opening-deadline first-acquire rule, pool `max = 4`, the
Dolt listener limits, the `after_connect` identity checks, the uncertain-write
fence, candidate retirement ordering. Nothing retries `PoolTimedOut`; no test
sleeps.

### Scope: this change reduces the family; it does not eliminate it

**This change reduces the runtime pool timeout family; it does not eliminate
it.** It fixes the three causes in our code that it names: D-a (every durable
write closed its SQL session), D-b (drop-then-reacquire raced SQLx's spawned
release) and D-c (the timeout was undiagnosable). Under the lead's elimination
rule it is not the family's resolution. The family's tracking item stays open,
and the PR description must say so in its first paragraph: "reduces, does not
eliminate".

### Decision D2 (deferred by the lead to a follow-up change): the ordinary acquire window

`ORDINARY_POOL_WINDOW` was sized for authenticating a pool, but through SQLx's
per-pool `acquire_timeout` it bounds every statement's acquire ahead of the
statement's own product budget (`QUERY_TIMEOUT`, 30 s). The 2 s window is a
runner-speed guess. The budget-derived shape is: once open, a statement's
acquire shares that statement's `QUERY_TIMEOUT`; opening-phase first
acquisitions keep the remaining startup deadline; `open_pool_budget_tests.rs`
changes from "ordinary window" to "statement budget".

Lead ruling, 2026-10-02 (binding): D2 is its own later change. This change
does not adopt D2 and does not lengthen any window; lengthening the 2 s window
is not a cure. Merging this change does not wait on D2. The typed acquire
diagnostic added here lets the next occurrence name its wait class, which is
the evidence the D2 design needs.

### Residual paths (family stays open)

Residuals 1 and 2 are the inferred path of the main failure (run 36930175874,
job 110597536829, partition 8, `mode_baseline_tests.rs:550`). This is an
inference from code and from the measured counts below, since nothing in that
log discriminates the waits (see "Evidence limits").

Measured per-pool new-connection authentications, one local run of
`mode_baseline_tests::all_four_modes_keep_builtin_dream_requests_and_own_memory_isolation`
(full table under "Measured exposure"):

| Pool | Before (origin/main) | After (two runs) |
|---|---|---|
| main (residual 2 plus one per open) | 80 | 23, 20 |
| dream candidates (residual 1) | 66 | 14, 14 |

Each of these connections still authenticates under the 2 s window. Remaining
exposures, each now reported by its wait class instead of a bare error:

1. Concurrent growth of a fresh pool: up to `max` (4) new connections
   authenticate under 2 s when concurrent work starts on a young pool (the
   dream's participants on its new candidate pool).
2. Candidate lifecycle writers (creation, promotion, abandon/transition,
   exclusion): 2-3 detached live-pool sessions per dream, each forcing one
   later new authentication on the live pool.
3. The idle-connection ping (`test_before_acquire`) under a Dolt stall.
4. The Dolt per-read timeout: an idle pooled session older than 30 s is
   closed by the server; the next acquire's ping fails and a new connection
   authenticates under 2 s (production after a pause; not a CI mechanism,
   every fixture pool is younger).
5. Permit waits behind a long statement or commit when more than `max`
   concurrent tasks share a pool (only when `max_parallel` is configured above
   4; with the default 4 and inline release, each task holds at most one
   connection at a time).

### Close and permit-budget consequences

- A reused write session holds its pool permit for the whole `apply`
  including `DOLT_COMMIT`; a detached one released its permit before `apply`.
  With inline release no permit is held by a floating release ping either, so
  on a candidate pool of `max` 4 with the default `max_parallel` 4 each
  participant holds at most one connection at a time and no permit wait
  occurs. No write path acquires a second connection from the same pool while
  holding its write session (`apply`, `apply_session_lifecycle` and
  `apply_change` take only `&mut MySqlConnection`), so holding the permit
  cannot deadlock.
- SQLx `Pool::close` waits for every permit. `MemoryStore::close` takes the
  write mutex first, so it never races a write. `Server::close`,
  `Server::close_installed_guard` (through `close_pools_and_owner`) and
  `Server::retire_pool` (candidate pool retirement, run under the write mutex
  by promotion and abandon) do not take it. When `Server::close` or
  `close_installed_guard` race an in-flight write session, the close now waits
  for that write up to `CLOSE_GRACE` (8 s) and then reaps the owned engine and
  drains again, where before the pool closed at once and the detached write
  died with the engine: same end state, different timing. The service owner
  shuts its store down through `MemoryStore::close` (write mutex first).
  `close_failed_open` and the migration-worker close run on the opening task
  after its own sequential work has stopped, so no write session is in flight
  there.

## Measured exposure

Per-pool new-connection authentications across one local run of
`mode_baseline_tests::all_four_modes_keep_builtin_dream_requests_and_own_memory_isolation`
(four modes, four dreams; macOS arm64, uninstrumented debug build; counted by
an uncommitted `eprintln!` at the authentication callback's entry, grouped by
branch class; candidate and promoting branches summed over the four dreams):

| Pool | Before (origin/main) | After (two runs) |
|---|---|---|
| main (store pools, max 4) | 80 | 23, 20 |
| usage ledger | 68 | 5, 4 |
| dream candidates | 66 | 14, 14 |
| promoting refs | 16 | 8, 8 |
| startup probe (max 1) | 6 | 6, 4 |

The remaining main-pool authentications are the candidate lifecycle
writers' detached sessions (residual 2) plus one per open; the remaining
candidate authentications are the participants' concurrent growth of each
fresh candidate pool (residual 1).

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `versioned-memory`: ADDED requirements "Pooled SQL session reuse" and
  "Diagnosed memory pool acquisition timeout".

## Impact

- `packages/kuru-memory/src/pool.rs` (new): `MemoryPool`, `PooledSession`,
  `PoolObservation`, `PoolAcquireTimedOut`, the `Executor` implementation and
  the test gate.
- `packages/kuru-memory/src/server.rs`: pools are `MemoryPool`; the per-pool
  observation is retained; first-connection inline release and verification on
  one session.
- `packages/kuru-memory/src/store.rs`, `store/usage_ledger.rs`,
  `store/export.rs`, `store/migrations.rs` (and its submodules),
  `store/stage_worker.rs`, `store/creation_template/hooks.rs`: pool type,
  read-transaction sessions released inline, `WriteSession` with
  `settle_receipted`.
- `Cargo.toml`, `packages/kuru-memory/Cargo.toml`, `Cargo.lock`: exact
  `sqlx-core = "=0.9.0"` (already locked) for the `Executor` stream helper.
- `docs/development.md`: reading a pool acquire timeout diagnostic.
- Error text at the three panic sites changes from the bare SQLx string to the
  typed diagnostic. No documentation cites the bare string.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
