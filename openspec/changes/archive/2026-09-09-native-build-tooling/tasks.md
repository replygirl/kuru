## 1. Native build tool isolation

- [x] 1.1 Keep native release builds limited to locked Rust and omit their repository-hook setup; verify the pinned mise setting and a clean isolated Rust-only build/package probe.
- [x] 1.2 Add Intel macOS and Linux arm64 native build/package PR checks and require them in ci-gate; verify Actionlint and require hosted results before merge as recorded in the verification ledger.
- [x] 1.3 Tighten release-note instructions, update operational documentation, run the full repository gate, record evidence and archive the change before the final commit.
