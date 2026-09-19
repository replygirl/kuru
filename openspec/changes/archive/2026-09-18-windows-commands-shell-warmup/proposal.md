## Why

Commit 68979a9 added a shared, Windows-only stock-PowerShell engine warm-up
(`kuru_connectors::shell_warmup::warm_up_stock_powershell_engine`) and wired
it into every `apps/kuru-tui` Windows test reaching the ToolHost stock
shell. Its own proposal named a gap and called it out of scope: two tests
in `packages/kuru-connectors/tests/windows_commands.rs` —
`stock_powershell_unicode_and_terminating_errors_are_observed` and
`retained_pinned_workspace_denies_replacement_and_keeps_shell_cwd` — also
construct a `ToolHost` and call `host.execute("shell", …)` (five calls
total), the exact cold-start-prone `shell_inner` path, but neither called
the warm-up. Both remain exposed to the same intermittent (~9% of Windows
shard runs) PowerShell 5.1 cold-start stall the parent change exists to
eliminate — main can still redden on Windows through this file.

## What Changes

Call `kuru_connectors::shell_warmup::warm_up_stock_powershell_engine()`
directly (the crate already owns this module; `windows_commands.rs` is an
integration test of the same package, built with `test-support` under
`--all-features`, so no dev-dependency feature-unification hop is needed)
at the top of both `windows_commands.rs` tests that reach the ToolHost
stock shell, before their first `host.execute("shell", …)` call. No change
to the warm-up helper itself, to the shell tool's own `timeout_ms` default
or validated range, or to any assertion in either test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None — this corrects test-harness coverage; no capability spec was wrong.

## Impact

- `packages/kuru-connectors/tests/windows_commands.rs`: adds a
  `use kuru_connectors::shell_warmup::warm_up_stock_powershell_engine;`
  import and one `warm_up_stock_powershell_engine().await.unwrap();` call
  at the top of `stock_powershell_unicode_and_terminating_errors_are_observed`
  and of `retained_pinned_workspace_denies_replacement_and_keeps_shell_cwd`.
  No other file changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
