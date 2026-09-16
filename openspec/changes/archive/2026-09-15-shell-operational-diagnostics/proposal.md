## Why

Built-in shell operational failures currently have different diagnostic paths on
Unix and Windows.  The Windows path first combines raw operation, cleanup, and
process-observation errors before projection, while the Unix path can discard
its partial capture and expose its raw error chain.

Those paths do not provide a stable cross-platform diagnostic boundary and can
make unrecognized implementation detail model-visible.  A failed shell still
needs a useful, bounded stderr clue without changing the normal completed
nonzero-exit result.

## What Changes

- Return one finite Kuru-authored operational shell category with a bounded,
  recognizable-secret-projected stderr diagnostic on Unix and Windows.
- Mark a stderr capture that has not reached EOF explicitly as pending and do
  not expose its partial bytes.
- Preserve the existing structured result for a fully captured child process,
  including a completed nonzero exit.

## Capabilities

### New Capabilities

### Modified Capabilities
- `provider-tools`: bounded, source-free built-in shell operational diagnostics.

## Impact

The change affects the connector built-in shell capture/diagnostic helpers,
their native-focused tests, and the shell protocol documentation.  It does not
change public tool result JSON, process ownership, cleanup, or redaction policy
APIs.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
