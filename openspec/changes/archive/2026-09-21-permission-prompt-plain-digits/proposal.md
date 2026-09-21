## Why

The TUI's permission modal (`apps/kuru-tui/src/ui.rs`, `View::key`) answers
only to Alt+1 through Alt+4; every other key, Esc included, falls through to
the global cancel-busy-turn handler. A live check found that terminals which
swallow or intercept the Alt modifier (tmux/cmux passthrough, several
terminal emulators, some remote sessions) deliver Alt+digit as a bare digit
or as nothing at all, so the prompt renders its choices but the user has no
key that produces them. The modal's own hint text also only ever named the
Alt form, so a user staring at an unresponsive prompt had no printed
alternative to try.

## What Changes

- While the permission modal is showing, the key handler accepts plain digit
  keys 1–4 as exact aliases of Alt+1–4, with identical gating: 2 and 3 only
  answer when the pending scope is rememberable, otherwise they are ignored
  with the same "Session and Always require the complete visible scope"
  notice Alt+2/Alt+3 already produce.
- The modal's own hint line names both forms compactly (e.g. "1 once · 2
  session · 3 always · 4 deny (also Alt+digit)") instead of only Alt+digit.
- No behavior changes when no permission modal is showing (plain digits stay
  ordinary composer input) and no other key (Esc, Enter) starts answering the
  prompt — this is additive key-binding coverage, not a change to permission
  semantics, scopes, or what each answer grants.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `apps/kuru-tui/src/ui.rs`: `View::key`'s permission-prompt branch now
  matches digit codes regardless of the Alt modifier; new unit tests cover
  the plain-digit alias, the not-rememberable gating, and that digits are
  ordinary input once the modal is gone.
- `apps/kuru-tui/src/ui/render.rs`: `draw_permission_prompt`'s choices hint
  text.
- `apps/kuru-tui/tests/terminal.rs`: three real-PTY tests updated to assert
  the new hint text instead of the retired Alt-only strings.
- `docs/usage.md`, `apps/kuru-docs/reference/configuration.md`: permission
  prompt key documentation.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
