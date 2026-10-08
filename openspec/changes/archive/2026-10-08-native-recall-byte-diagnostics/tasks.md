# Tasks

## 1. Exact native transcript diagnostic

- [x] 1.1 Preserve exact seven-turn full-string equality in `apps/kuru-tui/tests/support/windows_recall.rs`; identify the failing turn and its synthetic expected/actual byte lengths and Unicode scalars.
- [x] 1.2 Run formatting and Windows-target TUI lint; retain the native mismatch as pending acceptance without claiming a transport or composer fix.

Observed: formatting and Windows-target all-target/all-feature TUI lint pass.
Source review confirms the length guard and every full-string comparison remain
equivalent to the aggregate assertion. Prior native CI fails exact prompt bytes
on both Windows architectures; the differing value remains unknown until the
new diagnostic executes. No native pass or production correction is claimed.
