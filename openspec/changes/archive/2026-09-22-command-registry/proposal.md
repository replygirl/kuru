## Why

The interactive command list is currently repeated in a help string, local TUI branches, and runtime dispatch, while the editor has no slash-command completion. A command can be executable yet absent from help, and future Phase 2 handlers would have to keep those paths synchronized by hand. P09 and P11 need one dependable place to register working handlers as they arrive.

## What Changes

- Introduce one typed registry for working built-in TUI slash commands. Help, prefix completion, parsing, and dispatch use its names and metadata; modal review tokens remain private to their active prompts.
- Add complete, bounded editor completion for registered slash names, preserving modal input priority and ordinary composer digits.
- Implement `/clear` as a view-only clearing of the visible conversation surface and `/status` as a current-session projection using already available runtime facts. Neither command deletes project memory or starts a provider turn.
- Register later P01/P09/P11/P12 commands only when their own handlers are working. Progressive `SKILL.md` discovery and explicit custom commands follow in a separate P02 change on this registry; this slice does not claim their behavior.

## Capabilities

### New Capabilities

- `command-registry`: built-in TUI slash-command registration, consistent help/completion/dispatch, and working view-only commands.

### Modified Capabilities

## Impact

`apps/kuru-tui/src/ui.rs` and a small owning registry module, focused real-PTY tests, `docs/usage.md`, and `apps/kuru-docs/reference/commands.md`. No dependency, configuration schema, database migration, provider transport, or CLI command syntax change.

## Surfaces

- [x] interactive — slash help, completion, dispatch and visible session state
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
