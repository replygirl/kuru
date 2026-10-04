# Proposal

## Why

The kuru-runtime test waits in the priority subset are guessed numbers, 2 to 30 s,
below the product budgets of the operations they enclose: memory statements
(`QUERY_TIMEOUT`, 30 s), Remote managed replies (`OPERATION_TIMEOUT`, 35 s), owned
closes (`close_budget()`, 32 s), connector I/O (`IO_TIMEOUT`, 60 s) and web fetch
(20 s). A slow but progressing runner fails them while the product is inside its
own budget. The inventory (`tmp/roadmap/test-wait-inventory-2026-10.md` §7) counts
26 fix points; two are already fixed on main and one has a half assigned elsewhere,
leaving 24 fix points at 64 call sites. Each wait must end on its event under a
budget derived from the product value it encloses, with the derivation written at
the site. A passing run is unchanged: every wait is event-driven, so only the time a
real hang takes to report grows.

## Re-baseline

Measured on origin/main cbebf7b7, three commits past the inventory's 327f817c:
#206 (449dca9e, kuru-runtime tests), #211 (17c8aaeb, kuru-memory) and #204
(cbebf7b7, kuru-delivery). `git diff --stat 327f817c HEAD -- packages/kuru-runtime`
lists five files: `accounting_tests.rs`, `dream.rs`, `hook_tests.rs`,
`progress_wait.rs` and `step_timings.rs`. Every other site file is byte-identical
to the inventory's base, so its lines have not moved. The implementer re-checks each
line again at edit time.

- **Dropped, fixed by #206.** p1#24 (`accounting_tests.rs` 3543 and 3596 are now
  `join_on_progress` at 3551 and 3615 under `unhooked_gap_bound`; no
  `from_secs(10)` is left in the file) and p1#2 (`dream.rs` 790 and 1211 are now
  `join_on_progress` at 794 and 1224; no `from_secs(10)` is left in the file).
- **Excluded by agreement (assistant6).** The dream half of p1#23: the 5 s
  `observed.notified()` at `accounting_tests.rs:3607` in
  `abandoned_dream_keeps_usage_after_reopen_without_advancing_main`, with its
  failure explainer `explain_expired_dream_wait` (3654-3677; its `panic!` at 3673
  is the "~3673" the brief cites). This change does not touch those lines. The
  other half, the turn's first-usage wait at `accounting_tests.rs:3544`, stays.
- **Not fixed by #206, kept in full.** p1#36 and p1#51: #206 touched neither
  `review_tests.rs` nor `tests.rs`.
- **Moved.** p1#6 is now `dream.rs` 1043, 1053 and 1150 (was 1035, 1045, 1142) and
  p1#3 is `dream.rs:879` (was 871). The 30 s waits now at `dream.rs:783` and
  `:1211` are unrelated pre-cancel waits that #206 left unchanged and the
  inventory does not list in this subset.
- **Net.** 24 fix points, 64 call sites (69 - 2 for p1#24 - 2 for p1#2 - 1 for the
  excluded dream half).
- **Backend correction.** The inventory prices p1#6 at `QUERY_TIMEOUT` from its
  enclosure at :1142, which is Local. Sites :1043 and :1053 run on a managed
  Remote backend (`open_managed_observed`, `dream.rs:1015`), so their reply budget
  is `OPERATION_TIMEOUT`.

## Sites

Lines verified on cbebf7b7. "Derive from" is the intended source; the implementer
confirms it against the code and records what was done.

| Id | Where | Bound now | Derive from |
| --- | --- | --- | --- |
| p1#22 | `accounting_tests.rs:1783` | `timeout(100 ms, &mut running)`, an absence window | an observed cancellation-processed event, then one poll of the join (thin row) |
| p1#23 | `accounting_tests.rs:3544` (turn half only) | `timeout(5 s, observed.notified())` | `turn_admission_deadline()`: admission `memory.get` (engine.rs:1499) and the actor's reads before the provider stream |
| p1#19 | `dolt_tests.rs:888` | `timeout(10 s, written)` | `turn_admission_deadline()`: `undo_dream`'s reads then its persist write (dream.rs:521-567) |
| p1#6 | `dream.rs:1043`, `:1053` (managed Remote) | `timeout(20 s, ..)` barrier and sibling poll | `OPERATION_TIMEOUT` |
| p1#6 | `dream.rs:1150` (Local) | `timeout(20 s, ..)` staged select | `turn_admission_deadline()` |
| p1#3 | `dream.rs:879` | `timeout(30 s, whole fixture)` | `kuru_memory::test_support::fixture_deadline(1, 0)`: one fresh store lifecycle, its single-stall term (an owned close) and one statement |
| p1#68 | `engine.rs:6838`, `:6883` | `timeout(10 s, reached)` | `turn_admission_deadline()`: one `session_catalog_page` before the pause (engine.rs:4887-4888), a transaction under the store write lock running `DOLT_HASHOF('HEAD')`, `COUNT(*)` and the page query (kuru-memory store.rs:2669-2759) |
| p1#68 | `engine.rs:6786` | `timeout(10 s, reached)` | `turn_admission_deadline()`: `resume_session` reads before its pause (engine.rs:1183-1222) |
| p1#26 | `hook_tests.rs:1005` | `timeout(20 s, barrier.wait_sent())` | `OPERATION_TIMEOUT` |
| p1#26 | `hook_tests.rs:1028` | `timeout(20 s, turn)` join | `join_on_progress` under a gap of at least `OPERATION_TIMEOUT` |
| p1#27 | `hook_tests.rs:1008` | `timeout(15 s, poll)` | `OPERATION_TIMEOUT` |
| p1#32 | `hook_tests.rs:1731` | two `sleep 30` in the dream hook script | descendants block on a FIFO nobody writes (thin row) |
| p1#67 | `permission_tests.rs:372`, `:415` | `timeout(10 s, receiver.recv())` | `turn_admission_deadline()` |
| p1#58 | `progress_tests.rs:210` `stage()` (15 call sites) | `timeout(15 s, ..)` | `turn_admission_deadline()`, one edit |
| p1#59 | `progress_tests.rs:501` | `timeout(20 s, run)` join | `join_on_progress` under `unhooked_gap_bound` |
| p1#37 | `review_tests.rs:1286`, `:1292` | `timeout(10 s, notified)` | `turn_admission_deadline()` |
| p1#44 | `review_tests.rs:2501`, `:2508` | `timeout(10 s, yield_now spin)` | `turn_admission_deadline()` |
| p1#45 | `review_tests.rs:2528` | `timeout(2 s, abort, await, acquire_many(2))` | `QUERY_TIMEOUT` × `max_parallel` (2): each permit holder's `ledger.settle` write precedes its permit release, and the settles run one after the other on the store write lock, each under its own write budget (kuru-memory usage_ledger.rs:381, :386) |
| p1#35 | `review_tests.rs:512`, `:574`, `:702`, `:1173`, `:1380` | `timeout(30 s, event)` | `turn_admission_deadline()` |
| p1#35 | `review_tests.rs:1014` | `timeout(30 s, call_seen)` | the whole-turn budget: the HTTP MCP catalog build precedes the call, each RPC under connectors `IO_TIMEOUT` |
| p1#36 | `review_tests.rs:518`, `:600`, `:709`, `:823`, `:1019`, `:1178`, `:1295`, `:1385` | `timeout(10 s, task)` join | `join_on_progress`; `unhooked_gap_bound`, or `dream_gap_bound` where the fixture configures `cancelled_post_tool_hook()` (:709, :823, :1178) |
| p1#38 | `review_tests.rs:862`, `:1045` | `timeout(10 s, harness.shutdown(false))` | the shutdown guard (see What Changes) |
| p1#51 | `tests.rs:1032`, `:1236`, `:1633`, `:1796` | `timeout(10 s, task)` join | `join_on_progress` under `unhooked_gap_bound` |
| p1#52 | `tests.rs:1096` | `timeout(10 s, read request headers)` | `turn_admission_deadline()`, above web fetch's 20 s `FETCH_TIMEOUT` (crate-private, kuru-connectors `web_fetch.rs:24`) |
| p1#54 | `tests.rs:1219`, `:1763` | `timeout(10 s, oneshot)` | `turn_admission_deadline()` |
| p1#49 | `tests.rs:158` | `timeout(10 s, barrier.wait())` | `turn_admission_deadline()` |
| p1#55 | `tests.rs:1808`, `:1809` | `timeout(10 s, run_controlled(retry))` | `turn_admission_deadline()`: admission reads the stored journal, then refuses |
| p1#56 | `tests.rs:1826` | `timeout(30 s, harness.shutdown(false))` | the shutdown guard |
| p1#56 | `tests.rs:1830` | `timeout(30 s, memory.close())` | `close_budget()` |

## What Changes

- Budget sources, all existing: `turn_admission_deadline()` (`tests.rs:55`, the
  memory startup budget the Dolt listener derives its statement read timeout from),
  `kuru_memory::test_budgets::{QUERY_TIMEOUT, OPERATION_TIMEOUT, close_budget}` and
  `kuru_memory::test_support::fixture_deadline`, and the #181 and #206 helpers
  `progress_wait::{join_on_progress, TaskWatch, unhooked_gap_bound, dream_gap_bound}`.
  A wait that does not re-arm on progress takes the largest single product bound it
  encloses, the single-stall model `fixture_deadline` documents: a stalled step with
  its own bound reports its own error first. A join, which can re-arm, takes the
  #206 shape: `TaskWatch::attach(&mut harness)` before the spawn, then
  `join_on_progress` with the fixture's gap bound and a label in place of each
  `.expect(..)` text.
- Two product values the runtime cannot import are restated once, each citing its
  source line, in `tests.rs` beside `turn_admission_deadline()`, as kuru-tui's PR
  #212 did for the same values: the whole-turn budget, 600 s
  (`kuru-runtime/src/server.rs:190`) plus the 35 s post-cancel settlement (`:197`;
  docs/protocols.md, "up to 10 minutes", "up to 35 more seconds"), and connectors'
  `IO_TIMEOUT`, 60 s (`kuru-connectors/src/lib.rs:74`, `pub(crate)`; applied at
  `mcp.rs:1203` and `:1948-1949`).
- One shutdown guard replaces the three flat `harness.shutdown(false)` timeouts
  (`review_tests.rs:862`, `:1045`, `tests.rs:1826`), which is the three-site case
  for a shared helper: the longest single bound `shutdown` encloses, the tool host's
  concurrent MCP join at `IO_TIMEOUT` (`tools.rs:1678-1687`), plus one
  `QUERY_TIMEOUT` for the reconcile statements around it. The implementer counts
  `reconcile()`'s statements (`engine.rs:2441`) and states the figure.
- p1#22 (thin absence): wait for an observable that the cancellation was processed,
  then poll the join once; if no such observable exists without product code, the
  row is recorded not changed with that reason. p1#32 (thin keep-alive): the
  descendants block opening a FIFO nobody writes, as connectors' PR #215 did for
  its shell tests, so no descendant can exit on its own inside the join; a failure
  path releases them through the FIFO rather than signalling the published group by
  number, because AGENTS.md forbids authorizing termination from a reaped numeric
  identity.
- The one kuru-memory commit that adds `kuru_memory::test_budgets` under
  `test-support` is cherry-picked from PR #210 until it merges; see
  `blocking-changes.md`.

## What Does Not Change

- No product code. No edit to `progress_wait.rs`, `dream_gap_bound`,
  `quiesce_bound` or `turn_admission_deadline` (assistant6 relies on them
  unchanged). No change to `startup_timeout_secs`, no retry, and no weakening of
  isolation, ownership, recovery or the uncertain-write fence.
- The excluded dream half of p1#23 (`accounting_tests.rs:3607` and
  `explain_expired_dream_wait`).
- The assertions after each wait are byte-identical, except the messages that name a
  retired literal ("within 2s", "exceeded 30 seconds", "30 s sleep").
- No new helper beyond the shutdown guard above, which replaces three sites. No
  literal without a stated derivation.

## Remainder findings

Recorded so they are not mistaken for oversights; none is changed here.

- Inventory rows outside the priority subset in the same files: p1#39
  (`review_tests.rs:808`), p1#41 (`:866`), p1#43 (`:2479`), p1#46 (`:2546`), p1#50
  (`tests.rs:1016`, `:1224`), p1#53 (`:1117`) and p1#57 (`:1975`).
- `dream.rs:783` and `:1211`: 30 s pre-cancel waits that #206 left in place.

## Acceptance

Each of these passes locally on the final head, with its exit code recorded in
`tasks.md`:

- `mise run format:check` and `mise run typecheck`;
- `mise run //packages/kuru-runtime:lint` and `mise run //packages/kuru-runtime:lint:windows`;
- `mise run //packages/kuru-runtime:test`, run when no other real-memory suite from
  this session's worktrees is running;
- `mise run cospec -- validate runtime-derived-waits --strict`.

Per site, `tasks.md` records done (shape and source) or not changed (reason), and a
diff check shows no `from_secs` literal left at any done site. Local runs do not
reproduce a loaded CI runner; the PR's CI outcome is recorded separately and
labelled as such.

## Impact

- Files: `accounting_tests.rs`, `dolt_tests.rs`, `dream.rs` (test module),
  `engine.rs` (test module), `hook_tests.rs`, `permission_tests.rs`,
  `progress_tests.rs`, `review_tests.rs` and `tests.rs` under
  `packages/kuru-runtime/src`; this change's openspec directory; and PR #210's one
  cherry-picked kuru-memory commit.
- A hung test now reports after its derived bound instead of 2 to 30 s: about 30 to
  35 s for an admission or Remote wait, about 90 s for a shutdown guard, and about
  635 s for the one mid-turn MCP wait. Passing runs take the same time.
- Test only; no runtime, connector, platform, memory or documentation change.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
