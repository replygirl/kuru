## 1. Wire the warm-up into the two uncovered tests

- [x] 1.1 Add `use kuru_connectors::shell_warmup::warm_up_stock_powershell_engine;`
      to `packages/kuru-connectors/tests/windows_commands.rs` and call
      `warm_up_stock_powershell_engine().await.unwrap();` at the top of
      `stock_powershell_unicode_and_terminating_errors_are_observed`, before
      its first `host.execute("shell", …)` — verify `cargo test -p
      kuru-connectors --all-targets --all-features --locked` still compiles
      and passes on macOS (file is `#![cfg(windows)]`-gated, so this is a
      compile/lint-only check here; execution is Windows CI's job).
- [x] 1.2 Add the same call at the top of
      `retained_pinned_workspace_denies_replacement_and_keeps_shell_cwd`,
      before its `host.execute("shell", …)` call — verify the same command
      still compiles and passes.

## 2. Non-goals confirmed untouched

- [x] 2.1 Confirm `packages/kuru-connectors/src/shell_warmup.rs` (the warm-up
      helper itself) and the shell tool's own `timeout_ms` default/validated
      range in `packages/kuru-connectors/src/tools.rs` are unchanged — no
      diff to either file.
- [x] 2.2 Confirm no existing assertion in either touched test was removed
      or relaxed — the diff adds only an import line and one call per test.

## 3. Checks

- [x] 3.1 Run `mise run format:check`, `//packages/kuru-connectors:lint`, and
      `mise run typecheck`; record results.
- [x] 3.2 Run `cargo test -p kuru-connectors --all-targets --all-features
      --locked` and `mise run //apps/kuru-tui:test -- cli` /
      `-- trust` (regression check that the sibling, already-warmed tests
      still pass); record results.
- [x] 3.3 Run `mise run cospec -- validate windows-commands-shell-warmup
      --strict` and resolve or explicitly acknowledge any reported soft
      blockers before archiving.

Regression-test evidence: the actual named failure mode (an uncovered
`ToolHost`-shell test cold-starting past its timeout) is Windows-only and
intermittent (~9% of shard runs per the sibling investigations for the same
cold-start mechanism); it cannot be forced or reproduced on this macOS host.
The regression check available here is structural, not behavioral: before
this change, `stock_powershell_unicode_and_terminating_errors_are_observed`
and `retained_pinned_workspace_denies_replacement_and_keeps_shell_cwd` had
zero calls to `warm_up_stock_powershell_engine` (grep-verified); after this
change, both call it before their first `shell` execution (verified by
reading the diff and by 3.2's clean compile/pass on macOS, where the
Windows-gated call is inert). The real behavioral signal is the next several
Windows CI runs of `//packages/kuru-connectors:test`.
