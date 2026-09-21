## 1. Plain digits 1-4 answer the permission modal [critical]

- [x] 1.1 @regression (agent) revert `View::key`'s permission-prompt branch to its pre-fix `if key.modifiers.contains(KeyModifiers::ALT) {` gate, run `cargo test -p kuru --lib ui::tests::permission_prompt_accepts_plain_digits_as_alt_digit_aliases` -> FAILS: panic at `ui.rs:2614`, `assertion left == right failed / left: None / right: Some("/approval-once")` — a plain `1` with no modifiers is swallowed, exactly the live finding.
- [x] 1.2 @unit (agent) restore the fix, rerun the same test -> `test result: ok. 1 passed; 0 failed`; plain 1/2/3/4 each return `/approval-once` / `/approval-session` / `/approval-always` / `/approval-deny`.
- [x] 1.3 @unit (agent) same test with `rememberable = false` -> plain `2` and `3` each return `None`, `view.notice` becomes "Session and Always require the complete visible scope." (identical to Alt+2/Alt+3), and `view.input` stays untouched, not typed into the draft.
- [x] 1.4 @unit (agent) same test after `view.permission_prompt = None` -> a plain `2` is ordinary composer input again, `view.input == "2"`.
- [x] 1.5 @e2e (agent) real-PTY `real_pty_permission_choices_show_exact_file_scope_and_revoke_grants`, `real_pty_shell_permission_cancel_closes_reply_and_preserves_draft` and `real_pty_long_literal_permission_scope_survives_resize_and_inspection` in `apps/kuru-tui/tests/terminal.rs` continue to answer via `\x1b1`-`\x1b4` (Alt+digit byte sequences) end to end through a spawned `kuru` binary and a real mock provider -> `mise run //apps/kuru-tui:test` reports `19 passed; 0 failed; 1 ignored` for the `terminal` binary, proving Alt+digit still works unchanged alongside the new plain-digit alias.

## 2. Existing Alt+digit behavior and draft preservation are unchanged

- [x] 2.1 @unit (agent) `ui::tests::permission_prompt_preserves_draft_and_requires_visible_scope_for_remembering` -> `ok`: Alt+2/Alt+1 still answer, a non-digit char (`7`) still types into the draft while the modal is showing, Esc still cancels the turn (`/cancel`).
- [x] 2.2 @unit (agent) full `apps/kuru-tui` lib suite -> `cargo test -p kuru --lib` reports `80 passed; 0 failed`, no other `ui::tests::*` regressed.

## 3. Rendered hint text names both forms

- [x] 3.1 @unit (agent) draft-preservation test's 38-column `TestBackend` screen buffer (narrow layout) after the modal is drawn -> contains `"2 session"` and `"Alt+digit"`.
- [x] 3.2 @integration (agent) real-PTY assertions in `apps/kuru-tui/tests/terminal.rs` at the 80/120-column (wide) and 48-column (narrow) breakpoints, updated from the retired `"Alt+N Word"` strings -> pass under `mise run //apps/kuru-tui:test` (see 1.5).

## 4. Repo checks stay green

- [x] 4.1 @integration (agent) `mise run //apps/kuru-tui:test` -> lib suite `80 passed; 0 failed`; every `tests/*.rs` binary in the crate `0 failed`.
- [x] 4.2 @unit (agent) `mise run format:check` -> `Finished in 1.60s`, no diffs (after `cargo fmt --all`).
- [x] 4.3 @unit (agent) `mise run //apps/kuru-tui:lint` -> clean, `Finished in 12.04s`.
- [x] 4.4 @unit (agent) `mise run typecheck` -> `Finished in 4.64s`, no errors.
- [x] 4.5 @unit (agent) `mise run //apps/kuru-docs:check` -> build, format:check, lint and content checks pass; `Public docs artifacts, local links and anchors passed (/kuru/).`
- [~] 4.6 @integration (agent) full-workspace `mise run coverage` / `mise run check` -> defer: explicitly excluded by this unit's instructions; 4.1-4.5 plus this change's own tests cover the touched surface.
