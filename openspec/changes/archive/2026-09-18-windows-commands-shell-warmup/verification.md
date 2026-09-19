## 1. Both previously-uncovered tests now warm the engine before their first shell call [critical]

- [x] 1.1 @regression (agent) before this change (commit 68979a9), `grep -n "warm_up_stock_powershell_engine\|shell_warmup" packages/kuru-connectors/tests/windows_commands.rs` -> no matches: neither `stock_powershell_unicode_and_terminating_errors_are_observed` nor `retained_pinned_workspace_denies_replacement_and_keeps_shell_cwd` called the warm-up despite driving `ToolHost::execute("shell", …)` 5 times combined, the exact cold-start-prone path.
- [x] 1.2 @regression (agent) after this change, `grep -c warm_up_stock_powershell_engine packages/kuru-connectors/tests/windows_commands.rs` -> 3 (1 import + 2 call sites), and reading the diff confirms the call is the first statement in both test bodies, strictly before every `host.execute("shell", …)` call in each.
- [~] 1.3 @regression (agent) reproduce the actual named failure mode (a cold PowerShell 5.1 engine start stalling one of these two tests past its `tool shell` timeout) failing before this change and passing after, on a real Windows runner -> defer: intermittent (~9% of shard runs per the sibling installer/CLI-flake investigations for the same cold-start mechanism) Windows-only stall; this host is macOS, `powershell.exe` does not exist here, and cross-compiling `kuru-connectors` fails on an unrelated pre-existing host limitation (1.4), so it cannot be forced or disproven locally. The real signal is the next several Windows CI runs of `//packages/kuru-connectors:test`.
- [x] 1.4 @unit (agent) `cargo check --target x86_64-pc-windows-msvc -p kuru-connectors --all-targets --all-features --locked` -> fails identically with and without this change (verified via `git stash`): `aws-lc-sys`'s build script needs `<windows.h>` from the Windows SDK, unavailable cross-compiling from macOS — the same limitation `openspec/changes/archive/2026-09-18-windows-installer-warmup-and-timeout-attribution/verification.md` §1.3 recorded for `kuru-delivery` (shares the `reqwest` dependency edge). Confirms this is a pre-existing host limitation, not something this change introduced.

## 2. No weakened assertion, no widened timeout

- [x] 2.1 @unit (agent) code review of the diff -> the diff to `windows_commands.rs` is exactly one `use` line and two `warm_up_stock_powershell_engine().await.unwrap();` call lines; no existing assertion, `Config`, or shell command in either test changed, and `packages/kuru-connectors/src/shell_warmup.rs` / `packages/kuru-connectors/src/tools.rs` (the shell tool's `timeout_ms` default/validated range) show no diff.

## 3. Checks

- [x] 3.1 @unit (agent) `mise run format:check` -> clean.
- [x] 3.2 @unit (agent) `mise run //packages/kuru-connectors:lint` (clippy `-D warnings`, all targets/features) -> clean.
- [x] 3.3 @unit (agent) `mise run typecheck` -> clean across every workspace member.
- [x] 3.4 @unit (agent) `cargo test -p kuru-connectors --all-targets --all-features --locked` -> 199 passed, 0 failed in the library/unit suite; `tests/windows_commands.rs` reports "0 tests" on macOS (whole file `#![cfg(windows)]`-gated) — the same vacuous-but-expected compile/lint-level evidence the sibling change recorded, not execution evidence for the Windows-only tests themselves (see 1.3).
- [x] 3.5 @integration (agent) `mise run //apps/kuru-tui:test -- cli` -> `tests/cli.rs`: 12 passed incl. `cli_file_crud_and_shell_require_real_capabilities`; regression check that the already-warmed sibling tests are unaffected by this change.
- [x] 3.6 @integration (agent) `mise run //apps/kuru-tui:test -- trust` -> `tests/trust.rs`: 5 passed; same regression check as 3.5.
- [x] 3.7 @unit (agent) `mise run cospec -- validate windows-commands-shell-warmup --strict` -> recorded at apply/archive time.
