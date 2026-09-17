## 1. CLI parity fixture

- [x] 1.1 Update `apps/kuru-tui/tests/unix_shell_turn.rs` to validate unsigned `elapsed_ms` in each tool-observation event and normalize only that measurement before full debug/normal turn comparison.
- [x] 1.2 Run the exact Unix shell continuation test and scoped TUI typecheck, lint, format, and diff checks; record native macOS CI as pending on the corrected head.

Local evidence: the exact real-Dolt/HTTP `kuru_run_uses_the_owned_shell_and_preserves_the_responses_continuation` test passed (1/1) via `mise run //apps/kuru-tui:test -- ...`; `//apps/kuru-tui:typecheck`, `//apps/kuru-tui:lint`, `//:format:rust`, and `git diff --check` passed. Corrected-head native macOS CI remains pending; the earlier head failed the timing-sensitive comparison.
