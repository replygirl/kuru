## 1. Observe the streaming tool call in the runtime

- [x] 1.1 Replace the empty `ProviderEvent::ToolCallDelta` arm in `packages/kuru-runtime/src/progress.rs` with `ProgressObserver::tool_call`, which moves the round's activity from `RESPONDING` to `CALLING_TOOL` and republishes without reading the argument fragment; verify with `cargo test -p kuru-runtime --all-features --lib progress`.
- [x] 1.2 Extend the existing `tool_loop_replaces_preview_and_partial_call_has_no_authority` (it already drives a `ToolCallDelta`) to assert round 1's activity is `"Calling tool"`, that it carries no argument text, and that round 2 is back to `"Responding"`.

## 2. Name the in-flight tool in the facing view

- [x] 2.1 Add `View::calling_tool: Option<String>` in `apps/kuru-tui/src/ui.rs`, set from `Event::ToolStarted`'s raw catalog name when `actor == self.speaker_id`, cleared on the matching `Event::ToolSettled` and alongside every existing `preview = None` (`begin_operation`, `settle`, `complete_turn`, `/cancel`, `/quit`).
- [x] 2.2 Prefer `calling_tool` over `preview.activity` in `draw_preview` (`apps/kuru-tui/src/ui/render.rs`), keeping the existing prefix and truncation handling for the runtime-supplied label.
- [x] 2.3 Add `facing_tool_call_names_the_activity_and_clears_on_settlement` to `apps/kuru-tui/src/ui.rs`'s test module: a peer's `ToolStarted` is ignored, the facing one renders `"activity · Calling state_report"` in a drawn frame with no arguments, `ToolSettled` and `begin_operation` clear it.

## 3. Prove it in a real frame

- [x] 3.1 Add the `ToolActivityState`/`tool_activity_complete` fixture to `apps/kuru-tui/tests/terminal.rs`: one facing round whose only streamed output is a `response.function_call_arguments.delta` held open on a `watch` channel, completing on release into a `state_report` function call, then a plain final round.
- [x] 3.2 Add `real_pty_activity_line_tracks_a_streaming_tool_call` asserting `"activity · Calling tool"` in the held frame, the absence of `"Responding"` and of the argument sentinel on screen and in the PTY byte output, and no stale calling label after the turn settles.
- [x] 3.3 Verify the regression signature: revert `packages/kuru-runtime/src/progress.rs` and `apps/kuru-tui/src/ui/render.rs` only, confirm the real-PTY test FAILS showing `"activity · Responding"`, then restore and confirm it PASSES.

## 4. Gate checks

- [x] 4.1 `mise run format:check`, `mise run lint`, `mise run typecheck` -> pass.
- [x] 4.2 `mise run //apps/kuru-tui:test` and `mise run //packages/kuru-runtime:test` -> pass.
- [x] 4.3 `mise run cospec -- validate truthful-activity-label --strict`, `apply --json`, `mise run cospec:validate`, `mise run cospec:managed:check` -> clear.
