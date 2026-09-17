## 1. Early bootstrap diagnostics

- [x] 1.1 Add three fixed, flushed stderr markers to `packages/kuru-delivery/support/install.ps1` under its existing `-Verbose` switch; preserve genuine script execution, private publication checks, and the command deadline.
- [x] 1.2 Extend `apps/kuru-tui/tests/embedded_runtime.rs` to assert the early marker set on successful stock PowerShell bootstrap, then run scoped format, static, and bootstrap checks; record native Windows verification as pending until CI runs it.

Observed locally: `//:format:rust`, `//packages/kuru-delivery:lint:shell`, `//apps/kuru-tui:typecheck`, `//apps/kuru-tui:lint`, and the focused `bootstrap_inventory_is_direct_and_bounded` app test passed. Stock Windows PowerShell execution and the new direct-stderr assertion remain pending native CI; macOS has no stock PowerShell 5.1.
