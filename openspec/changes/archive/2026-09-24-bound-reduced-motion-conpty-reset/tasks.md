## 1. Reduced-motion terminal acceptance

- [x] 1.1 Update `apps/kuru-tui/tests/support/windows_terminal.rs` and `tests/windows_terminal.rs` so the existing native ConPTY case accepts at most one standalone inert style reset over the original observation interval, checks unchanged cells/styles and terminal state, and keeps focus-loss byte silence strict.
- [x] 1.2 Extend the existing `apps/kuru-tui/src/ui.rs` disabled-motion unit across multiple clock ticks and verify it never advances an idle animation frame.
- [x] 1.3 Run the focused unit, owning TUI lint and formatting locally; record the native Windows ConPTY case as a required final-head CI gate until it actually passes.

Local evidence: `ui::tests::ambient_clock_ignores_editing_and_preserves_busy_focus_and_static_behavior` passed 1/1 on macOS; owning TUI all-target Clippy, Rust formatting, strict Cospec validation and `git diff --check` passed. The corrected ConPTY assertion is Windows-only and remains **unrun** locally. Exact corrected-head Windows CI must pass before #80 merges; the prior main job `107894979861` is the failing-before observation, not a corrected result.
