## 1. Accept plain digits in the key handler

- [x] 1.1 In `View::key`'s permission-prompt branch (`apps/kuru-tui/src/ui.rs`),
  match digit codes regardless of the Alt modifier instead of gating the
  whole block on `KeyModifiers::ALT`, and verify Alt+1-4 and plain 1-4 share
  the same match arms (`/approval-once`, `/approval-session` and
  `/approval-always` only when `rememberable`, `/approval-deny`).
- [x] 1.2 Verify the not-rememberable gate (`self.notify(...)` + consumed
  `None`) fires identically for plain 2/3 and Alt+2/Alt+3, and that no other
  key (Esc, Enter, non-digit chars) changes behavior.

## 2. Regression coverage

- [x] 2.1 Add
  `ui::tests::permission_prompt_accepts_plain_digits_as_alt_digit_aliases`
  covering: plain 1-4 match their Alt+digit outcomes; plain 2/3 are ignored
  (with the notice, not typed into the draft) when not rememberable; plain
  digits are ordinary composer input once no modal is showing. Confirm it
  fails against the pre-fix `ALT`-gated branch and passes with the fix (see
  verification.md 1.1-1.2).
- [x] 2.2 Extend the existing
  `permission_prompt_preserves_draft_and_requires_visible_scope_for_remembering`
  test's rendered-screen assertion for the new hint text and verify with
  `cargo test -p kuru --lib ui::tests`.

## 3. Hint text

- [x] 3.1 Update `draw_permission_prompt`'s `choices` strings
  (`apps/kuru-tui/src/ui/render.rs`) to name both forms compactly at both the
  wide and narrow width breakpoints, and verify the rendered screen contains
  the new text in the unit test above.
- [x] 3.2 Update the three real-PTY tests in `apps/kuru-tui/tests/terminal.rs`
  that asserted the retired `"Alt+N Word"` hint substrings, and verify with
  `mise run //apps/kuru-tui:test`.

## 4. Docs

- [x] 4.1 Update `docs/usage.md`'s terminal-controls table and prose to
  describe answering the prompt with `1-4` or `Alt+1-4`.
- [x] 4.2 Update `apps/kuru-docs/reference/configuration.md`'s permission
  section the same way, and verify with `mise run //apps/kuru-docs:check`.

## 5. Verify and rebuild

- [x] 5.1 Run `mise run format:check`, `mise run //apps/kuru-tui:lint`,
  `mise run typecheck`, `mise run //apps/kuru-tui:test` and
  `mise run //apps/kuru-docs:check`, and verify all pass (see
  verification.md 4.1-4.5).
- [x] 5.2 Rebuild the binary with `mise run build` and record its absolute
  output path.
