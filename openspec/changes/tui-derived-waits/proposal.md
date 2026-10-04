# Proposal

## Why

The kuru-tui test waits `READY_TIMEOUT` (10 s, about 180 call sites) and
`EXIT_TIMEOUT` (5 s, 48 sites) are guessed numbers below the 35 s reply deadline
(`OPERATION_TIMEOUT`, `packages/kuru-memory/src/service/rpc.rs:38`, applied at
`rpc.rs:185`) that bounds every managed-Remote memory call the PTY child makes:
its memory commands, and `/quit`, which runs `shutdown(true)` and so
`reconcile()` over Remote (`engine.rs:1069` and `:1094`, `facade.rs:2447`). A
slow but progressing runner fails them while the product is inside its own
budget. The same shape sits in the fixed 1 s `wait_exit` drain, in the server,
CLI and `runtime_tests` waits, and in a few thin animation and absence windows:
18 fix points. Each wait must end on its event under a budget derived from the
product value it encloses. A passing run is unchanged, because every wait is
event-driven; only the time a real hang takes to report grows.

## Sites

Measured on origin/main 449dca9e, one commit past the inventory's 327f817c. That
commit touches only `packages/kuru-runtime` and `openspec`; `git diff --stat
327f817c HEAD -- apps/kuru-tui` is empty, so no kuru-tui line moved and no site
is already fixed. The implementer re-checks each line before editing.

| Id | Where (verified on 449dca9e) | Bound now | Derive from (product value it undercuts) |
| --- | --- | --- | --- |
| t1#2 | `tests/support/terminal.rs:19` `READY_TIMEOUT`, with dependents t1#11, #14, #26 (108), #27 (15), #34, #40, #41, #49 and the `startup_timeout` addend (:114) | 10 s literal | `OPERATION_TIMEOUT` (35 s) plus a frame allowance from `ui.rs:358` and `:2583` |
| t1#23 | `tests/terminal.rs:48` `EXIT_TIMEOUT`, with t1#25 (48 `wait_exit(EXIT_TIMEOUT)` lines, identical list) | 5 s literal | the `/quit` path: Remote reconcile(s) at `OPERATION_TIMEOUT` plus a frame allowance, plus the owner close (`close_budget()`, `server.rs:171`) only if the child's exit waits on it (task 2.2) |
| t1#21 | `tests/support/terminal.rs:588` `wait_exit` drain (51 call sites) | 1 s literal | reader EOF/EIO under a cleanup budget derived from the longest legitimate slave holder after exit |
| t2#41 | `tests/support/windows_terminal.rs:11` `READY`, with t2#34 (`windows_terminal.rs:536`), #35 (:574-577), #36 (:578-579), #39 (16 uses), #52 (support :509) | 10 s literal | the same value as `READY_TIMEOUT`, restated for the Windows module |
| t2#59 | `tests/cli.rs:733` `terminal.wait(.., 15 s)` | 15 s | sequential OAuth discovery requests, each under connectors `IO_TIMEOUT` (60 s, `lib.rs:74`, `http.rs:9`) |
| t2#60 | `tests/cli.rs:748` `terminal.wait_exit(15 s)` | 15 s | token exchange plus MCP host shutdown join, each under `IO_TIMEOUT` (`mcp.rs:254`, `mcp.rs:1203`) |
| t2#25 | `tests/server.rs:101` `recv_timeout(10 s)` | 10 s | memory startup budget (`memory.startup_timeout_secs`, 30 s default, `config.rs:597`; `service.rs:964,1031`) |
| t2#28 | `tests/server.rs:185` reqwest `timeout(10 s)` | 10 s | A2A request budget (600 s, `kuru-runtime/src/server.rs:190`; 35 s settlement :197) |
| t2#29 | `tests/server.rs:244,254` `now + 5 s` after SIGINT | 5 s | server shutdown: `harness.shutdown(false)` (`cli.rs:1590`) reconcile(s) at `OPERATION_TIMEOUT` |
| t2#16 | `tests/unix_shell_turn.rs:33` `CLI_TIMEOUT`, with t2#18 (:352), #22 (:797,804), #23 (:829) | 45 s literal | memory startup budget plus the provider's per-request and completion budgets (`IO_TIMEOUT` 60 s, `COMPLETION_TIMEOUT` 600 s) |
| t2#82 | `src/ui/runtime_tests.rs:1011` `timeout(10 s, loop_task)` | 10 s | in-process Local backend: `QUERY_TIMEOUT` (30 s, `store.rs:66`) through `write_deadline()` (`store.rs:7609-7611`) |
| t2#78 | `runtime_tests.rs:106` `wait_for_request`, with t2#79 (:601) and t2#83 (:1029) | 10 s, 5 s, 15 s | `QUERY_TIMEOUT` per write before the provider call (`actor.rs:571-572`, `facade.rs:1634-1638`) |
| t2#80 | `runtime_tests.rs:862` notice poll | 10 s | `QUERY_TIMEOUT` (`memory_notice.rs:68`, `store.rs:3624`, `:3792-3797`) |
| t1#43 | `tests/terminal.rs:2639` `read_for(700 ms)` then `assert_eq!(unchanged, reduced)` | 700 ms window | observable repaint (animated); barrier or deferral (reduced) |
| t1#44 | `tests/terminal.rs:2647` `read_for(400 ms)` then `assert_no_output_since` | 400 ms window | barrier frame, or deferral |
| t2#9 | `tests/embedded_runtime.rs:564,569,570` `sh -c "/bin/sleep 6 & wait"`, 2 s deadline, 200 ms grace | 2 s race | observed fork marker, or deferral |
| t2#32 | `tests/windows_terminal.rs:265,281` `read_for(700 ms)` | 700 ms window | `wait` on `output.len() > before` |
| t2#33 | `tests/windows_terminal.rs:275,353` `quiet(450 ms)` and `quiet_with_optional_style_reset(450 ms)` | 450 ms window | barrier probe frame, or deferral |

Counts: the 48 `wait_exit(EXIT_TIMEOUT)` lines, the 127 `READY_TIMEOUT` code lines
cited by t1#26, t1#27, t1#34, t1#40, t1#41 and t1#49 (the only other grep hits are
the import at `terminal.rs:46` and a comment at `:2673`) and every product line
spot-checked in the site list's enclosure chains match on this base. A per-shape
regex count of `wait_composer_frame(.., READY_TIMEOUT)` gives 106 where the list
says 108; the cited line list is the evidence, and the per-shape count is approximate.

PR #208 adds 15 lines inside `Terminal::spawn` (`support/terminal.rs` 174-210).
Inference, not measured: once it merges, `wait_text` (:352), `submit` (:440) and the
`wait_exit` drain (:588) move down by 15; the constants at :19 and
`startup_timeout` (:107-115) sit above the hunk and do not move. This change edits
no line in 174-210.

## What Changes

- `tests/support/terminal.rs` derives `READY_TIMEOUT` once, at its definition, from
  `kuru_memory::service::OPERATION_TIMEOUT` plus a frame allowance, with the
  derivation in a comment there. `startup_timeout()` (which adds `READY_TIMEOUT`)
  therefore grows by the same amount, and so do `trust.rs`'s uses through the shared
  module. This is the intended effect of one definition, not a widening, and
  `startup_timeout_secs` itself does not change.
- `tests/terminal.rs` derives `EXIT_TIMEOUT` once, at its definition, from the
  `/quit` shutdown path, with its figures shared with `startup_timeout()` rather
  than restated a third time.
- `wait_exit` ends its drain on reader EOF/EIO under a derived cleanup budget instead
  of a flat 1 s; the same constant may serve `LATE_OUTPUT_WINDOW`.
- `tests/support/windows_terminal.rs` derives `READY` the same way, and its dependents
  follow. It is linted for the Windows target only; it runs natively in CI.
- Product values that are not public (`IO_TIMEOUT`, the runtime's inline 600 s A2A
  request budget, `QUERY_TIMEOUT`) are restated at one test constant each, citing the
  defining file and line and the docs page that documents it. No visibility changes in
  kuru-connectors or kuru-runtime.
- The CLI, server, `unix_shell_turn` and `runtime_tests` waits take their bounds from
  those constants, each with its derivation at the site or in the constant's comment.
- Thin windows (t1#43, t1#44, t2#9, t2#32, t2#33) end on an observable event where a
  few lines of test support allow it; otherwise the site is recorded as deferred in
  `tasks.md` with a code-backed reason.
- The one kuru-memory commit that makes `OPERATION_TIMEOUT` public under
  `test-support` is cherry-picked from PR 1 (`test/memory-derived-waits`) until that PR
  merges; see `blocking-changes.md`.

## What Does Not Change

- No product code. `apps/kuru-tui/src` changes only inside `#[cfg(test)]`
  `src/ui/runtime_tests.rs`. No change to `startup_timeout_secs`, no retry, and no
  weakening of isolation, ownership, recovery or the uncertain-write fence.
- Remainder rows stay as they are unless they sit on a line already being edited.
  Known ones, recorded so they are not mistaken for oversights: `runtime_tests.rs:884`
  (the same `timeout(10 s, loop_task)` shape as :1011), `terminal.rs:2637` (the 4100 ms
  read before the window), `support/terminal.rs:662` (`Drop`'s 1 s `close`), the Windows
  `EXIT` (30 s) and `windows_terminal.rs:162-163` (the Windows `startup`
  arithmetic), `server.rs:155` (the Windows twin's 80 s) and `server.rs:175`.
- No new helper unless it replaces three or more sites. No literal without a stated
  derivation. No visibility change in `kuru-memory` beyond PR 1's one commit.
- The assertions after each wait are byte-identical.

## Impact

- Files (the nine owned by the site list): `apps/kuru-tui/src/ui/runtime_tests.rs`,
  `tests/cli.rs`, `tests/embedded_runtime.rs`, `tests/server.rs`,
  `tests/support/terminal.rs`, `tests/support/windows_terminal.rs`,
  `tests/terminal.rs`, `tests/unix_shell_turn.rs`, `tests/windows_terminal.rs`; this
  change's openspec directory; and PR 1's one cherry-picked commit.
- A hung test now reports after its derived bound, several times longer than before
  (from 35 s to several minutes, set per site, instead of 5 to 45 s), bounded in CI by
  the existing native job `timeout-minutes`. Passing runs take the same time.
- `server.rs` and `unix_shell_turn.rs` may include `support/terminal.rs` under
  `#[cfg(unix)]` (with `#[allow(dead_code)]`, as `trust.rs` does) to reach the shared
  constants; that is a test-module include, not a new helper.
- Windows-only files cannot run here. They must pass `mise run
  //apps/kuru-tui:lint:windows`, and run natively in PR CI only.
- Test only; no runtime, documentation or dependency change.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
