# Tasks

## 1. Shared prelude and contract

- [ ] 1.1 Add `packages/kuru-delivery/tests/support/stock_powershell.rs` with the exact two-line `$PSHOME` import prelude and a prefixing helper, and verify both kuru-delivery and kuru-tui tests compile it through `#[path]`
- [ ] 1.2 Add a portable contract test in `packages/kuru-delivery/tests/powershell_diagnostics.rs` asserting byte parity with both `install.ps1` import lines and passing #98's discovered-command helper over a prefixed script, and verify it passes in `mise run //packages/kuru-delivery:test`

## 2. Apply to test-launched stock shells

- [ ] 2.1 Prefix the source-entrypoint probe and the PE-inspection launcher in `apps/kuru-tui/tests/windows_cli.rs`, and verify the probes' statements and assertions are otherwise unchanged in the diff
- [ ] 2.2 Prefix the completion-activation command in `apps/kuru-tui/tests/embedded_runtime.rs`, and verify its 100 s bound and assertions are unchanged
- [ ] 2.3 Prefix the architecture probe in `packages/kuru-delivery/tests/bootstrap_windows.rs`, and verify the module-autoload refusal probes remain prelude-free
- [ ] 2.4 Verify `mise run format:check`, `mise run lint`, `mise run lint:windows` and `mise run typecheck` pass

## 3. Native evidence (CI-only)

- [ ] 3.1 (CI-only) Windows native tests: the source-entrypoint, PE-inspection, completion-activation and bootstrap architecture tests pass with the prelude
- [ ] 3.2 (CI-only, measured over later runs) Install-timeout family occurrences after this change are appended to the flake catalogue with run and job ids
