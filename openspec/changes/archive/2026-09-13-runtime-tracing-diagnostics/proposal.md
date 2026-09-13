## Why

Kuru has no structured operational trace for a running turn, provider retry, or
tool invocation. A bounded per-project diagnostic record makes failures
supportable without exposing prompts, credentials, or private memory.

## What Changes

- Add runtime and connector tracing at turn, actor, tool, and typed retry boundaries.
- Add a fixed-size, owner-private JSONL diagnostic ring for one runtime-owning CLI process and a global `--debug` operational-detail flag.
- Keep subscriber setup in the app, machine stdout/TUI output unchanged, and all payload/error-chain logging excluded.

## Capabilities

### New Capabilities
- `operational-diagnostics`: bounded redacted local tracing and diagnostic files for runtime-owning operations.

### Modified Capabilities
- `provider-tools`: typed provider retry observations without admitting remote diagnostics.

## Impact

Workspace-pinned `tracing` and `tracing-subscriber` dependencies and Cargo.lock; app subscriber/ring and CLI flag; runtime actor/engine spans; connector retry events; focused fixtures and user documentation. No migration, public `Event`/`TurnOutput` change, remote export, environment logging control, or credential-store access.

## Surfaces

- [x] interactive — `--debug`, bounded local diagnostics, and unchanged TUI/stdout behavior.
- [ ] deploy — no deployment topology changes.
- [x] integration — pinned tracing APIs and existing provider retry classifications.
- [ ] agent-behavior — no prompt, tool, or model-routing policy changes.
