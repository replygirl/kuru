# Tasks

## 1. Diagnosis record

- [ ] 1.1 Record in the PR body the placeholder semantics (`view.busy` selects "Keep your next thought here…", otherwise "What shall we explore or build?") and the sibling site list: `apps/kuru-tui/tests/terminal.rs` smoke sites asserting the welcome placeholder under `READY_TIMEOUT` (around the `/mem` tab-completion backspace, the `focus draft` backspace and the paste `\x01\r` clear) and `Terminal::command` in `tests/support/terminal.rs`; verify by grepping for `What shall we explore or build?` and confirming every hit is covered or justified.

## 2. Support helper

- [ ] 2.1 Add a helper in `apps/kuru-tui/tests/support/terminal.rs` that waits for the activity line to leave `dream · pool`, bounded by the passed sandbox startup timeout, with the screen in the expiry diagnostic; verify the diagnostic contains the captured screen when it expires.
- [ ] 2.2 Use it at every affected site in `apps/kuru-tui/tests/terminal.rs` and in `Terminal::command` before the welcome-placeholder assertion; verify no remaining post-dream placeholder assertion relies on a bare `READY_TIMEOUT` and no assertion after settling changed.

## 3. Evidence

- [ ] 3.1 Add a deterministic test (or record mutation evidence) in which a screen shows `dream · pool` with the busy placeholder; verify the old shape fails and the helper waits then passes.
- [ ] 3.2 Run `mise run //apps/kuru-tui:test` for the affected PTY tests and record observed results; name unrun checks and reasons.
