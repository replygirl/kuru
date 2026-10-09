# Proposal

## Why

Native ConPTY fixtures send plain UTF-8, which can synthesize Release-only Alt-code characters (observed for U+0301 on Windows x64 and ARM). That differs from Windows Terminal committed-key input. Ratatui recommends Kuru's existing Press-only filtering to avoid duplicate keys. Upstream PR745 deliberately discusses Release-only Alt-code commitment; changing the dependency is unjustified for this coverage work.

## What Changes

- Remove the rejected vendored dependency, Cargo patch and all associated coverage/formatter/repository exceptions. Keep latest released crossterm 0.29.0 from the registry.
- Correct only the owned recall/key fixture host: observe ConPTY's documented Win32-input-mode request and send committed Unicode and explicit command key records over the existing pipe.
- Preserve exact decomposed Unicode, ordinary release filtering and all seven durable prompts. Qualify keyboard proof separately from unresolved raw-text paste/Alt-code behavior.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Windows native test helpers and acceptance documentation only. Production EventStream and Press-only behavior remain. No vendoring, fork, unreleased pin, downgrade, replacement backend, timing workaround, coverage exception or HTML roadmap edit.

## Surfaces

- [x] interactive — exact Windows committed-key recall acceptance
- [ ] deploy
- [x] integration — released crossterm and documented ConPTY host input
- [ ] agent-behavior
