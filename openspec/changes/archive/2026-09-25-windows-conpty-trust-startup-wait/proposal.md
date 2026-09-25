## Why

The Windows ConPTY trust fixture gives an approved launch only a ten-second preflight wait, although that path starts managed memory before checking its deliberately missing Responses key. A cold startup can therefore make the test fail before it observes the specified pre-TUI refusal.

## What Changes

- In `apps/kuru-tui/tests/windows_terminal.rs`, use the fixture's existing configured startup bound only after the persistent trust choice, then require the exact missing-key diagnostic. Keep the short declined-choice wait, no-alternate-screen assertion, and durable trust-status check.

## Impact

This changes one Windows application test's approved-path wait and diagnostic assertion. Product behavior, memory startup limits, workflows, and supported platforms are unchanged. Exact-head native Windows CI is unrun for this correction and remains a hard premerge gate.
