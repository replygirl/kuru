## 1. Reliable terminal execution [critical]

- [x] 1.1 @regression (agent) Replay delayed real terminal interaction before and after the fix -> original fixture with a one-second delayed exit redraw failed at child.wait(timeout=5), matching main run 34378748501; repaired real flow passes with the same delay, restored attributes and persisted conversation.
- [x] 1.2 @regression (agent) Delay terminal output beyond the old drain window, then exceed the PTY buffer -> real child emits 1 MiB after 700 ms; every byte and final marker are read before successful exit. A separate stalled child fails within its deadline with screen diagnostics.
- [x] 1.3 @integration (agent) Run all terminal fixtures -> chat, selectors, ambient/static motion, cancellation, persistence and terminal restoration pass: seven CLI integration tests and four preference tests, including all three PTY fixtures.

## 2. Repository delivery

- [x] 2.1 @integration (agent) Run mise run check -> mise run check exits 0: 148 Rust tests, 11 installer tests, two Python PTY-driver regression cases, 97.64% line coverage (6731/6894), lint/format/strict specs/managed drift pass.
- [x] 2.2 @manual (agent) Inspect branch diff and independent review -> independent agent review found no blocking issue; all application source is byte-identical to main and no check or coverage requirement was weakened.

Hosted PR checks and the resulting main run are publication follow-through;
their final results will be reported from GitHub after merging the reviewed fix.
