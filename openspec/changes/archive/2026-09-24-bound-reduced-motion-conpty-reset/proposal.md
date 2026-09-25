## Why

Windows CI observed one standalone ANSI style reset after a completed reduced-motion frame. The rendered cells and cursor stayed unchanged, but the byte-for-byte quiet assertion treated that inert reset as continued animation.

## What Changes

- In `apps/kuru-tui/tests/support/windows_terminal.rs`, add a reduced-motion observation that accepts zero bytes or one exact style reset only when every rendered cell and terminal state remains unchanged; retain the existing strict focus-loss check.
- In `apps/kuru-tui/tests/windows_terminal.rs`, use that observation for the existing ConPTY reduced-motion case.
- In `apps/kuru-tui/src/ui.rs`, extend the existing unit test across several disabled-motion clock ticks.

## Impact

Test code only; no terminal renderer, runtime behavior, dependency, or workflow change. The native Windows ConPTY case remains the post-push verification gate.
