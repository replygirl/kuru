# Proposal

## Why

Measured: PR #181 run 37076544504 (head da4033ed on main f958287a; that PR touches only kuru-runtime tests), macos-latest coverage partition 4, job 111067851181: `real_pty_accepts_chat_navigation_commands_and_restores_terminal` in `apps/kuru-tui/tests/terminal.rs` failed with `screen contains ["What shall we explore or build?", "enter send"] ... timed out after 10s; process is still running`. The captured screen showed `⠋ dream · pool · 10s`, a mid peer-round activity log and the composer placeholder "Keep your next thought here…".

Source diagnosis: `src/ui/render.rs` (composer) renders "Keep your next thought here…" when `view.busy` and "What shall we explore or build?" otherwise. The status line renders `{status}  ·  {operation_ms/1000}s` while busy, and `Event::Dream` sets the status label to `dream · <actor>` (`src/ui.rs`). So the placeholder is a busy indicator: the welcome placeholder is only correct once the dream has finished. The tests wait for the first composer frame under `sandbox.startup_timeout` and then assert the welcome placeholder under the flat `READY_TIMEOUT` (10 s, `tests/support/terminal.rs`).

Inference, not measured: a startup dream was still running when the composer first appeared and, under coverage on a loaded runner, outlasted the 10 s guess. The bound was chosen at runner speed rather than by the event the assertion depends on. That the dream in the failing run was the startup dream (rather than a post-turn dream) is inferred from the elapsed `10s` and the absence of any sent input; the run's own logs were not otherwise cross-checked.

## What Changes

- `apps/kuru-tui/tests/support/terminal.rs`: add a helper that waits for the activity line to leave `dream · pool` (the dream-completion event), bounded by the caller-supplied sandbox startup budget, with the captured screen in the expiry diagnostic. `Terminal::command` uses it where it awaits the welcome placeholder.
- `apps/kuru-tui/tests/terminal.rs`: use the helper before each assertion of the welcome placeholder that follows the startup composer frame under `READY_TIMEOUT` (the smoke test's `What shall we explore or build?` sites after backspacing a draft, and after paste/clear).
- A deterministic test (or recorded mutation evidence) showing the old shape fails while a dream is shown and the new helper waits for it.
- No change under `apps/kuru-tui/src`, no new constant, no raised `READY_TIMEOUT`, no retry, and no change to what is asserted once the dream has settled.

## Impact

Test and test-support code in `apps/kuru-tui` only. CI time is unchanged when the dream finishes promptly; on slow runners the wait extends to the existing startup budget instead of failing at 10 s.
