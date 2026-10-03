# Verification

## 1. A failing command whose cleanup also fails reports both causes [critical]

- [x] 1.1 @regression (agent) `cargo test -p kuru --lib --all-features --locked cleanup_combination_tests` against the old inline body (commit c4591c68, `finish` = `let value = primary?; cleanup?; Ok(value)`), then the fixed body -> Observed 2026-10-02, macOS arm64. Red at c4591c68: `both_failures_name_the_primary_then_the_cleanup_cause` FAILED with `left: "execute: tool refused"` vs `right: "execute: tool refused; tool host cleanup failed: MCP shutdown: child not reaped"` (3 passed, 1 failed). Green at the fix commit: 4 passed, 0 failed; the message holds both causes, primary before cleanup, joined by `; `.
- [x] 1.2 @unit (agent) the same module covers `(Ok, Ok)`, `(Ok, Err)`, `(Err, Ok)` -> Observed 2026-10-02: the success value is returned; a single failure is returned unchanged (`MCP shutdown: child not reaped`, `execute: tool refused`), passing both before and after the fix.
- [~] 1.3 @e2e (agent) a `kuru tool`/turn/dream run whose tool host or harness shutdown fails while the primary fails -> defer: no existing fixture can make shutdown fail through the CLI. The only stdio MCP fixture (`apps/kuru-tui/tests/cli.rs`, `[mcp.stdio]`) asserts the child is never started; the `kuru-connectors` `test-support` seams on `ToolHost` cover parallel reads and web fetch routing only; `HookHost::quiesce`, `McpHosts::shutdown`, `ShellRegistry::shutdown` and `MemoryStore::close` have no failure injection. Per scope, no new seam was added; the combinator is shared by all eight sites, so the unit test covers the combination each site performs.

## 2. Existing CLI behavior is preserved

- [x] 2.1 @integration (agent) `mise run //apps/kuru-tui:test` (full package: lib, every integration test binary) -> Observed 2026-10-02, macOS arm64: exit 0 in 390 s. lib 134 passed; tests/cli.rs 43 passed, 1 ignored; embedded_runtime 7; terminal 43 (1 ignored); trust 22; unix_shell_turn 5, including the assertion that a successful turn prints `diagnostic cleanup failed; diagnostics may be incomplete`; all other binaries pass.

## 3. Static checks

- [x] 3.1 @integration (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run cospec -- validate --all --strict` -> Observed 2026-10-02, macOS arm64: each exits 0 (lint 118 s; root lint:windows ran `//apps/kuru-tui:lint:windows` for x86_64-pc-windows-msvc, 81 s; typecheck 71 s; validate "0 errors, 0 warnings").

## 4. Adjacent drops left unchanged (recorded, out of scope)

- [~] 4.1 @manual (agent) the provider turn serializes its JSON result with `?` before `harness.shutdown`, and `kuru serve` returns from `serve(..).await?` before `shutdown(false)` -> defer: neither is a combination site; fixing them changes when cleanup runs, which this change keeps fixed. Candidates for a follow-on unit.
