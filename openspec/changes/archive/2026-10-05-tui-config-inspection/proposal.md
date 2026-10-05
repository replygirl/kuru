# Proposal

## Why

People can inspect the merged configuration from the CLI, but an interactive session has no working `/config` command. When a choice or setting behaves unexpectedly, users need to leave the conversation and reconstruct which layer supplied the effective value. The TUI already holds the immutable configuration snapshot and current project selections, so it can show that information without rereading files or activating new authority.

## What Changes

- Add `/config` to the working TUI command registry, help, completion, and local dispatch.
- Show the captured effective configuration with final-leaf source provenance and clearly separate current saved/live mode, model, and effort selections.
- Add a bounded, read-only projection from `ConfigSnapshot` that uses captured origins, applies already-loaded project preferences, retains successful live selections separately, and redacts secret-bearing configuration values while keeping environment-reference names visible.
- Show a visible notice whenever the bounded inspection omits values or rows.
- Document `/config` and its captured-versus-live distinction in the command reference.

## Capabilities

### New Capabilities

### Modified Capabilities
- `command-registry`: define `/config` as a working local inspection command with safe output and no provider or configuration-file activation.

## Impact

The TUI command registry, command dispatch, view state, CLI-to-TUI snapshot handoff, focused tests, command reference, and `kuru-core`'s `ConfigSnapshot` inspection projection are affected. The projection is read-only and uses the existing immutable snapshot and already-loaded project preferences; it introduces no configuration mutation, authority re-review, credential-store access, dependency, or persistent-data migration. Secret-bearing MCP environment/header values, URL userinfo, and sensitive URL query values are redacted; configured environment-reference names remain visible without resolving them.

## Surfaces

- [x] interactive — a user-visible TUI command and configuration view
- [ ] deploy — no deploy or runtime topology change
- [ ] integration — no third-party contract
- [ ] agent-behavior — no prompts, tools, model routing, or agent output change
