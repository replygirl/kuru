## Why

The Windows native delivery test still reads `Message.content`, a field removed
by the typed-message foundation. That test target therefore fails to compile on
Windows before it can verify that the installed application retains the original
user input.

The stored transcript remains a typed message with one text block. The failure
is limited to the stale test assertion, which must inspect the supported text
projection without weakening the delivery acceptance check.

## What Changes

- Replace the obsolete `content` field assertion with an exact `plain_text`
  assertion for the expected one-text user message.
- Keep the existing read-only installed-memory and transcript integrity checks.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None.

## Impact

- `packages/kuru-delivery/tests/support/mise_acceptance.rs`
- Windows native delivery test compilation and its typed transcript assertion

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
