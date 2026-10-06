# Proposal

## Why

The current TUI reports tool activity and outcomes but does not provide collapsible result cards or checked file diffs. Users need to distinguish simultaneous calls, inspect safe output and understand managed changes without exposing private actor context or confusing streamed proposals with admitted effects.

## What Changes

- Add bounded current-session tool cards keyed by final admitted call context, with stable admission order and truthful pending, completed, denied, failed, cancelled and unavailable states.
- Provide a private runtime presentation seam for bounded external-tool output and checked file receipt identity; retain metadata-only presentation for cognitive and peer tools.
- Selectively recover chunk-safe shell previews and connector-owned bounded diffs of the recorded checkpoint snapshots.
- Preserve the existing public event, journal and TurnOutput contracts. Replay does not redispatch tools and explicitly marks missing transient output or admission metadata unavailable.
- Preserve composer/modal controls and confirmed compaction notices. Document transient card details and explicit checkpoint pruning.

## Capabilities

### New Capabilities

- `tool-cards`: admitted tool identity, bounded collapsible safe output, partial progress, replay and practical terminal behavior.

### Modified Capabilities

- `verified-file-edits`: checked read-only diff projection of retained applied receipt snapshots.
- `provider-tools`: bounded chunk-safe partial shell presentation without altering dispatch, projection, cancellation or cleanup.

## Impact

TUI `ui.rs`, `ui/render.rs`, `ui/scene.rs` and owning tests; connector `file_edits.rs`, `tools.rs`, `redaction.rs`, `shell_diagnostic.rs`, Unix/Windows shell execution boundaries and owning tests; narrow runtime tool dispatch/observation, presentation exports and tests; owning command/tool documentation. No database schema, durable event/outbox, provider route, new dependency, memory migration, journal/TurnOutput field or dream/reconciliation policy change. N3 remains separately owned.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
