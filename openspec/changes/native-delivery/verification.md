## 1. Native behavior equivalence [critical]

- [x] 1.1 @equivalence (agent) Run the migrated Rust terminal and protocol tests -> 14 TUI integration cases and 29 connector tests passed on 2026-09-09. Real PTYs cover delayed redraw, backpressure/stalls, cancellation, saved preferences and terminal restoration; Rust subprocesses preserve JSON-RPC/Codex/MCP failure and framing coverage.
- [x] 1.2 @regression (agent) Exercise native installer safety and update -> 13 archive tests, 7 delivery CLI cases and 2 app updater tests passed. Verified execution/replacement succeeds; corrupt, ambiguous, oversized and unsafe archives preserve existing bytes. Packaging preserves input/output on rejection, including hardlink/symlink aliases. Isolated source install at /tmp/kuru-native-source-install ran kuru 0.1.0 successfully.
- [x] 1.3 @equivalence (agent) Build/check docs and release helpers through package mise tasks -> npm-local docs build and 22 native validator tests passed; 14 release/Cocogitto/Communiqué tests passed through actual subprocess/HTTP boundaries. All Python files and root language projects were removed; Node is owned only by the docs app.
- [x] 1.4 @integration (agent) Run standalone cospec without external JS dependencies -> the known duplicate-entry defect is addressed by the narrowly scoped package preload documented in design. Actual standalone tests pass clear JSON, strict validation, hard/soft/missing-artifact blockers, and embedded resolution; mise apply and managed no-drift checks pass.

## 2. Delivery

- [x] 2.1 @integration (agent) Run the complete gate -> mise run check exited 0 on 2026-09-09: 205 Rust tests passed, including actual fixture tools, with 97.55% line coverage (8435/8647). Formatting, Clippy, metadata, shell/actions, docs and cospec gates passed. No application exclusions or threshold changes were introduced.
- [ ] 2.2 @runtime (agent) Run GitHub CI on macOS and Ubuntu -> required checks exercise the native package tasks on actual hosted runners.
