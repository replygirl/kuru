## Context

`View::key`'s permission-prompt branch gated the whole digit match on
`key.modifiers.contains(KeyModifiers::ALT)`. Any terminal, multiplexer or
remote session that intercepts or drops the Alt modifier (tmux/cmux
passthrough, several emulators) never produces a `KeyEvent` with that flag
set for the digit row, so the branch is skipped and the digit falls through
to ordinary composer-input handling — the prompt renders but nothing the
user can reliably press answers it. Esc is deliberately excluded (it already
means "cancel the turn"), so digits are the only remaining way in.

## Goals / Non-Goals

**Goals:**
- A plain digit 1–4 answers the prompt exactly like Alt+digit, including the
  2/3-requires-rememberable gate and its notice.
- No change to what a modal-showing key does when it isn't a digit 1–4, and
  no change to key handling when no modal is showing.

**Non-Goals:**
- Any new answer key (Esc/Enter stay unbound to the prompt).
- Any change to grant scopes, remembering, or the permission engine itself —
  this only widens which keystroke reaches the existing `/approval-*`
  commands.

## Decisions

- **Drop the `ALT` gate entirely rather than add a parallel plain-digit
  branch.** The match is already keyed on `key.code`, which is identical
  (`KeyCode::Char('1'..='4')`) whether or not Alt is held; the modifier only
  ever narrowed which codes were considered. Removing it makes Alt+digit and
  plain digit share one code path by construction, so "same outcome as
  Alt+digit" is structural rather than something a second branch could drift
  out of sync with. Rejected: mirroring the whole block behind
  `key.modifiers.is_empty()` — it duplicates the gating logic and the notice
  call, and the two copies could diverge under a future edit.
- **Hint text switches from "Alt+1 Once" to "1 once … (also Alt+digit)".**
  The compact form fits the existing width budget (62-column breakpoint) in
  both the wide and narrow layouts and states the plain-digit form as
  primary, since it is now the more portable one; Alt+digit remains named so
  existing muscle memory still resolves.

## Operational surface

No bind address, container topology or secret is introduced. The interactive
surface is the existing `kuru` binary's terminal UI: `View::key` (an in-memory
state transition, no I/O) and `draw_permission_prompt`'s rendered hint text,
exercised by the crate's own `ratatui::backend::TestBackend` unit tests and by
the real-PTY fixtures in `apps/kuru-tui/tests/terminal.rs` that already spawn
`kuru` as a local child against a local mock `responses` server.

## Risks / Trade-offs

- [A future feature might want to type a leading digit into the composer
  while a permission prompt is visible] → Digits 1-4 were already special
  when Alt was held; this only extends that existing carve-out to the
  no-modifier case, and every other digit (5-9, 0) is untouched.
- [Existing PTY/visual snapshot tests assert the old "Alt+N Word" strings] →
  Updated deliberately in this change (`apps/kuru-tui/tests/terminal.rs`,
  `apps/kuru-tui/src/ui.rs`'s own render assertion) rather than left to
  silently pass on substring coincidence.
