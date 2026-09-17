## 1. Windows cancellation fixture

- [x] 1.1 Change `apps/kuru-tui/tests/windows_terminal.rs` to return an SSE `response.completed` event from its gated Responses handler, preserving the delayed and fresh markers. Independent source review confirmed valid framing and the unchanged release gate.
- [x] 1.2 Verify the fixture retains its cancellation, next-draft, no-late-result, and persisted-session assertions; run scoped static checks and record native Windows execution as pending CI. Existing assertions are unchanged; TUI typecheck/lint, Rust format, and diff checks passed locally. The Windows-only ConPTY fixture has not run locally and requires native CI on the corrected head.
