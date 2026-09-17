## Why

The CLI shell-continuation test compares separate debug and normal turns byte-for-byte after removing session identity. Typed tool observations include elapsed milliseconds, so otherwise equivalent runs can differ in one measured timing field.

## What Changes

- In `apps/kuru-tui/tests/unix_shell_turn.rs`, require each tool-observation event to carry a valid unsigned elapsed-millisecond value, then omit only that value for the existing whole-turn parity comparison.
- Keep all shell continuation, receipt, diagnostics, and output assertions unchanged.

## Impact

Only a test comparison changes. Native macOS and Ubuntu coverage will verify the fixture; production event timing and JSON output remain unchanged.
