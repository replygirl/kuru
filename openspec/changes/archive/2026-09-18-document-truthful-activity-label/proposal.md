## Why

`docs/usage.md`'s activity-label paragraph was rewritten in 6c03789 without a
cospec change, and it still describes the pre-fix behavior: it says the label
"clears back to `Responding` when the call settles," which was true only once
the next streamed fragment arrived, not at the moment of settlement, and it
never mentions more than one facing tool call being in flight at once. Follow-up
fixes to `apps/kuru-tui` (`packages/kuru-runtime/src/progress.rs` callers in
`apps/kuru-tui/src/ui.rs` and `apps/kuru-tui/src/ui/render.rs`) made the label
clear truthfully at settlement and track parallel in-flight calls; the doc
needs the matching wording.

## What Changes

- `docs/usage.md`: reword the activity-label sentence describing settlement so
  it states the label returns to `Responding` the moment every in-flight call
  has settled (never lagging on a stale name), and add that when more than one
  of the speaker's calls are in flight at once, the label names whichever was
  dispatched most recently.

## Impact

Readers of `docs/usage.md`'s "The live interface" section relying on the
activity label's documented behavior. No code, schema, or `apps/kuru-docs`
changes.
