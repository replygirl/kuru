# Tasks

## 1. Causal native input observation

- [x] 1.1 Add `input-events` mode in `tests/fixtures/windows_terminal.rs` with the public terminal session and actual EventStream; retain all character/release/modifier observations, existing wait bounds, stream/session restoration and outer mode receipt.
- [x] 1.2 Add a bounded test in `tests/support/windows_recall.rs` sending the exact decomposed draft through existing ConPTY byte transport; finish and release native ownership before inspecting the event receipt, with exact scalar diagnostics and unchanged seven-turn assertion.
- [x] 1.3 Run formatting and Windows-target TUI lint, review bounded native cleanup, and name the native causal observation as pending until CI actually executes it.

Observed checks: root Rust formatting and Windows-target TUI lint passed. Independent review confirmed readiness after terminal setup, capture before filtering, bounded observations, restoration and owned cleanup before byte comparisons. Native execution is pending CI; this probe cannot by itself distinguish ConPTY console translation from crossterm decoding, and no production cause or fix is claimed.
