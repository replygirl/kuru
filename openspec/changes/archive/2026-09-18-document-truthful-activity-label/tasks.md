## 1. Update docs/usage.md

- [x] 1.1 Reword the activity-label settlement sentence in `docs/usage.md`'s
      "The live interface" section so it states the label returns to
      `Responding` the moment every in-flight call has settled, and add that
      the label names whichever of several in-flight calls was dispatched
      most recently; verified every sentence against
      `apps/kuru-tui/src/ui.rs` (`View::event`'s `ToolStarted`/`ToolSettled`
      arms) and `apps/kuru-tui/src/ui/render.rs` (`draw_preview`'s activity
      line).

## 2. Verify

- [x] 2.1 Ran `mise run format:check`; `docs/usage.md` needed no reflow.
