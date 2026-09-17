## Why

The native Windows cancellation fixture still returns a JSON Responses body, while the streaming provider now requires an SSE terminal event. Its fresh turn fails protocol validation before the test can observe the intended recovery behavior.

## What Changes

- Update the `/v1/responses` handler in `apps/kuru-tui/tests/windows_terminal.rs` to return the same delayed and fresh completed outputs as valid `text/event-stream` terminal events.
- Keep the release gate and cancellation, next-draft, output, and session assertions intact.

## Impact

Only the Windows ConPTY test fixture changes. Native Windows application coverage must confirm the recovered turn; other platform behavior and production code are unchanged.
