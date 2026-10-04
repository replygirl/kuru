# Proposal

## Why

The kuru-connectors tests wait on flat literals (1 s to 15 s) that sit below the
product budgets the waited-for operation runs under: `IO_TIMEOUT` (60 s),
`COMPLETION_TIMEOUT` (600 s), the auth refresh allowance, the shell timeout and
cleanup allowance. A slow runner then fails a test while the product is still
inside its own budget. One site already did: the 15 s bound on the isolated MCP
child (`mcp.rs:4416`), which encloses `IO_TIMEOUT`-bounded requests, failed in PR
#212's run 37180709856 (macOS partition 2, job 111373013191, "isolated MCP
environment fixture timed out"). Other sites are thin races or absence checks, where a fixture
sleep races a short product bound, so a stall flips the outcome or passes the test
for the wrong reason. This change fixes 29 priority fix points so each wait ends on
its event under a budget derived from the product value it encloses. PR #212 waits
for this change to land.

## Sites

Measured on origin/main 449dca9e, one commit past the inventory's 327f817c. That
commit (#206) touches only `packages/kuru-runtime` and `openspec`; `git diff --stat
327f817c HEAD -- packages/kuru-connectors` is empty, so no connectors line moved and
no site is already fixed. Every cited line printed the cited code. Two small
corrections to the briefs: the brief header says `mcp.rs` "~4464" for c1#129, but
the site list and the base agree on 4416; and the `timeout_ms: 3_000` lines that c1#37
cites as 4799 and 4863 are at 4798 and 4862 (the cited lines are the closing
`}),` and `)`). The implementer re-checks each line at edit time.

Visibility measured on this base (see Policy below): `crate::IO_TIMEOUT` is
`pub(crate)`, `COMPLETION_TIMEOUT` and `STREAM_IDLE_TIMEOUT` are `pub`, and
`auth::tests` is a child of `auth`, so it can read `manager.inner.http_timeout`. The other product values these sites need are private to a module
that is not an ancestor of the test module (`CALLBACK_TIMEOUT` in `auth::login`,
`DIAGNOSTIC_TIMEOUT` in `providers::diagnostics`, `LOCK_DEADLINE` from `mcp::tests`),
are inline literals (`tools.rs:1344`, `:1346-1347`), or sit in a file PR #207 edits
(`CLEANUP_ALLOWANCE` in `unix_shell.rs:36`; rpc `CLEANUP`, `GRACE`, `STDERR_DRAIN` in
`rpc.rs:50-52`).

| Id | Where (verified on 449dca9e) | Bound now | Derive from (source; I = imported, R = restated once with citation) | Status |
| --- | --- | --- | --- | --- |
| c1#111 | `src/auth/tests.rs:172` `callback_request` (callers 257, 346, 385, 1204, 1297) | 5 s read | reply path = callback read (`CALLBACK_TIMEOUT`, `login.rs:17,52`, R) + token exchange (`http_timeout` = `IO_TIMEOUT`, `auth.rs:119`, `http.rs:98`, I) + reply write (`CALLBACK_TIMEOUT`, `login.rs:235`) | unchanged on base |
| c1#117 | `auth/tests.rs:702,748` gate `reached` | 10 s | the refresh allowance, `manager.inner.http_timeout` (I): `refresh_rejected` `auth.rs:240-247`, timer `auth.rs:293-297`, `retry.rs:91-110` | unchanged |
| c1#118 | `auth/tests.rs:799` `&mut refresh` | 15 s | same allowance: the refresh future ends at its own deadline, so this wait ends no later | unchanged |
| c1#120 | `auth/tests.rs:872` `BeforeFirst` gate | 5 s | same allowance, in both iterations (`http_timeout` is 2 s in the `expire` one: tie with the product deadline, same failure either way) | unchanged |
| c1#110 | `auth/tests.rs:94` `Fixture::arrived` (callers 393, 1051, 1093) | 5 s | `self.manager.inner.http_timeout` (token POST timeout `http.rs:98`) | unchanged |
| c1#127 | `src/mcp.rs:3560,3571,5400` | 5 s | 3560: `LOCK_DEADLINE` (30 s, `mcp_credentials.rs:25,86-91`, R), the only budget enclosed before the revocation request reaches the fixture. 3571: `IO_TIMEOUT` (the revocation request, `http.rs:9`, I) plus `LOCK_DEADLINE` (the loop's own `acquire` waits for the settlement's lease). 5400: `IO_TIMEOUT` as a named upper bound on the peer start the first `initialize` (`mcp.rs:1565,1609`) waits on (inference; no product budget encloses a process start) | unchanged |
| c1#129 | `mcp.rs:4416` isolated child | 15 s | `initialize` and `tools/list` (2 x `IO_TIMEOUT`, `mcp.rs:1565,1609,1948`) + `McpHosts::shutdown` join (`IO_TIMEOUT`, `mcp.rs:1203`) (I). The peer's one rustc compile in the child (`test_support.rs` `fixture_binary`) has no product budget | unchanged; failed in #212 CI |
| c1#131 | `mcp.rs:5211` (peer hold at `:5138`) | 1 s over a 400 ms hold | `IO_TIMEOUT` (I) for the wait; the slow peer is held on a release-file gated step until the independent alias returns | unchanged |
| c1#8 | `src/mcp_credentials.rs:1538` | `sleep(75 ms)` then `!is_finished()` | causal: poll the pinned `acquire` once while the lock is held (`futures` is a dependency), then drop the holder and await it (bounded by `LOCK_DEADLINE`, nameable in this file) | unchanged |
| c1#79 | `src/providers.rs:1801,1810` | 2 s | `COMPLETION_TIMEOUT` (I): the stream timer (`providers.rs:1112-1117`) encloses connect (10 s), POST and the first chunk wait (`STREAM_IDLE_TIMEOUT`, 300 s) | unchanged |
| c1#85 | `providers.rs:2251,2267` | server 150 ms vs client 80 ms | release gate after the generic deadline is observed client-side (a later probe request on the same generic client times out first); outer 3 s at `:2272` follows `provider.completion_timeout` | unchanged |
| c1#89 | `providers.rs:2293,2302` | server 150 ms vs 80 ms | release gate after the 80 ms completion deadline is observed (a `complete()` on the same provider times out); the `:2308` `timeout(1 s, server)` follows | unchanged |
| c1#90 | `providers.rs:2303` | 1 s | `IO_TIMEOUT` (I): `models` timer `providers.rs:1061` and per-request `:1069` | unchanged |
| c1#133 | `providers.rs:2751` | server `sleep(3 s)` | hold the socket on a gate released after `complete()` returns; returning proves the diagnostic bound (`providers/diagnostics.rs:10,280`) fired; the `< 3 s` assert goes | unchanged |
| c1#91 | `providers.rs:2789` | dribble 4 x 700 ms | dribble until a gate released after `complete()` returns; the 700 ms cadence stays below `DIAGNOSTIC_TIMEOUT` (2 s, R in a comment); the `< 2600 ms` assert goes | unchanged |
| c1#95 | `providers.rs:3032` | 2 s | `COMPLETION_TIMEOUT` (I): `collect_completion` `providers.rs:294`, send loop `:612-634`, retry delays bounded by `retry.rs` | unchanged |
| c1#96 | `providers.rs:3044` | `sleep(1 s)` then count | causal, no sleep: the aborted task's join proves the retry future was dropped; the exact request count after the continuation, with `unexpected fixture request` on a replayed reply, proves no replay | unchanged |
| c1#99 | `providers/subscription_tests.rs:1118,1124,1313,1336,1385` | 5 s | `provider.completion_timeout` (I; `COMPLETION_TIMEOUT` 600 s as built by `subscription`, `providers.rs:545-551`) | unchanged |
| c1#102 | `subscription_tests.rs:1191` fixture guard (3 callers) | 5 s | `COMPLETION_TIMEOUT` (I): the client's total deadline, after which it drops the connection | unchanged |
| c1#103 | `subscription_tests.rs:1225` | 5 s around `released.await` | drop the timer around `released.await` (ends on the test's release); guard accept/read and each write phase separately under `COMPLETION_TIMEOUT` | unchanged |
| c1#104 | `subscription_tests.rs:1272,1281` | 2 s | `COMPLETION_TIMEOUT` (I), as c1#79 | unchanged |
| c1#64 | `src/test_support.rs:251` `wait_for_requests` (callers `rpc.rs:1078`, `mcp.rs:4897,5202,5321`) | 5 s | `IO_TIMEOUT` (I): `Rpc::request` timer `rpc.rs:545,555,1068-1071`. The `rpc.rs:1078` caller changes through the helper; `rpc.rs` is not edited | unchanged |
| c1#37 | `src/tools.rs:4785,4852` (`timeout_ms` at 4798, 4862) | `exec sleep 5` vs 3 s | a command that blocks on an unwritten FIFO (`mkfifo`, then `exec cat` of it) instead of the finite sleep; `timeout_ms: 3_000` stays | unchanged (cited lines off by one) |
| c1#44 | `tools.rs:4803,4865,5096,5212,5285` | 6 s | shell-only sites (4803, 4865, 5096, 5285): `ShellRegistry::shutdown` returns by its own deadline of `CLEANUP_ALLOWANCE` (5 s, `unix_shell.rs:36,577-600`, R) plus its 10 ms poll and a written allowance. 5212 (shell + MCP): `McpHosts::shutdown`'s `IO_TIMEOUT` timer (`mcp.rs:1203`, I) is the larger branch of the `ToolHost::shutdown` join | unchanged |
| c1#40 | `tools.rs:5087,5180,5270` | 2 s | the shell timeout the test passes: 120_000 ms, the product maximum (`tools.rs:1346-1347`, R once as a const used for both the `timeout_ms` and the wait) | unchanged |
| c1#41 | `tools.rs:5156,5163,5196` | 1 s | the same const (starting shell passed 120_000 at `:5151`; `unix_shell.rs:475` deadline). 5196 has no budget of its own: it falls under that shell's deadline (inference) | unchanged |
| c1#53 | `tools.rs:5767` isolated child | 10 s | the child's shell default timeout (30 s, `tools.rs:1344`, R) + `CLEANUP_ALLOWANCE` (5 s, R) | unchanged |
| c1#10 | `src/web_fetch.rs:269` `FIXTURE_TIMEOUT` (dependents 341-422, 381, 438, 563, 729) | 1 s | `FETCH_TIMEOUT` (20 s, `web_fetch.rs:24`, nameable through `use super::*`): the fixture serves a fetch whose budget the test passes (at most 2 s, `:688`) and the product caps at `FETCH_TIMEOUT`; the `+ 1 s` addends at 381 and 438 are derived from the longest fixture `delay` (200 ms, `:699,:721`) | unchanged |
| c1#7 | `tests/fixtures/stdio_peer.rs:142` `Step::Sleep` | sleep as keep-alive | add `Step::Park` (drain stdin without recording until the product closes it) and a release-file gated step (c1#131); replace `Step::Sleep` at `mcp.rs:4866,5302`; keep `Step::Sleep` and its `rpc.rs:1000,1043` call sites | unchanged |

## Policy

- Imported means the test module names the product value directly. Restated (R)
  means a private or inline value, or one in a PR #207 file, is written once per
  test module as a named const whose doc comment cites its source file and line on
  449dca9e, the way the sibling `tui-derived-waits` change restates the connectors'
  `IO_TIMEOUT`. No `src/` product line changes, so there is no visibility widening
  (`pub(crate)`) in this change: the briefs' "no product code change" covers it.
  A restated value cannot be checked against its source by the compiler; the citation
  is the only link. If that is not acceptable, the alternative is a separate
  exposure commit like PR 1's `test_budgets`, which needs its own scoping.
- No flat wait decides an outcome. A wait ends on its event under a derived budget
  with the derivation at the site or constant. No retry, no change to
  `startup_timeout_secs`, no weakening of isolation, ownership, recovery or the
  uncertain-write fence. No new helper unless it replaces three or more sites.

## What Changes

- `tests/fixtures/stdio_peer.rs` gains a `park` operation and a release-file gated
  operation; `src/test_support.rs` gains the matching `Step` variants (the
  exhaustive match in `StdioFixture::with_cache` needs an arm each) and derives
  `wait_for_requests` from `IO_TIMEOUT`. Existing steps behave as before; `Step`
  has only in-crate consumers.
- The 27 other fix points change in `#[cfg(test)]` modules only, per the table:
  auth, MCP, MCP credentials, providers and subscription tests, tools and web fetch.
- HTTP fixtures that answered late on a sleep (c1#85, c1#89, c1#133, c1#91) answer on
  a gate the test releases after the product call returns.
- Not changed: `hooks.rs`, `rpc.rs`, `unix_shell.rs` (PR #207), any product code, the
  assertions after each wait (apart from the elapsed-time asserts that a gate makes
  redundant at c1#133 and c1#91), and every remainder row below.

## Remainder findings

Recorded so they are not mistaken for oversights; none is changed here.

- `rpc.rs:1000,1043` `Step::Sleep(5_000)` call sites (c1#7's other users; PR #207's file).
- Waits next to a site, same shape, outside the 29: `auth/tests.rs:193,717,822,887,937,944,1007`;
  `tools.rs:4787,4793,4855,4861` (5 s ready and 10 s execute waits in the c1#37 tests)
  and the `exec sleep 5` keep-alives at `:5082,5175,5265` (cancelled by shutdown);
  `mcp.rs:5323` (8 s shutdown) and the 5 s cleanup drains after `:4416` and
  `tools.rs:5767`; `providers.rs:1827,2235,2277,2308,2729,3101` and
  `subscription_tests.rs:1129,1163,1298,1318,1346,1390` (server-join waits).
- `ToolHost::shutdown` also joins `hooks.quiesce()`; c1#44's sites configure no hook
  (inference from the test bodies read), so that branch adds nothing there.

## Impact

- Files: `src/auth/tests.rs`, `src/mcp.rs` and `src/mcp_credentials.rs` test
  modules, `src/providers.rs` test module, `src/providers/subscription_tests.rs`,
  `src/test_support.rs`, `src/tools.rs` test module, `src/web_fetch.rs` test module
  and `tests/fixtures/stdio_peer.rs` under `packages/kuru-connectors`, plus this
  change's openspec directory.
- A hung test now reports after its derived bound (up to 60 s to 600 s) instead of
  1 to 15 s; passing runs take the same time or less (c1#131 no longer holds on a
  sleep). c1#133 and c1#91 still take about the 2 s diagnostic deadline when they
  pass, but a regression that removes the diagnostic bound is now reported only when
  the completion deadline (600 s) fires, since the elapsed asserts go with the sleeps.
  Decision for the implementer: keep a derived elapsed check (the diagnostic deadline
  plus one dribble interval, restated) beside the gate if fast failure is wanted.
- The stdio fixture is also the Cargo binary `kuru-connectors-stdio-fixture`
  (`test-support`); it is checked by `//packages/kuru-connectors:format:fixture:check`
  and linted for the Windows target. The Windows peer is Cargo-built, the Unix one
  is compiled by `rustc` from the same source at first use.
- Test only; no runtime, documentation or dependency change.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
