## Why

Windows native fixtures showed that a checked private-stage removal can return `Uncertain` after it has requested deletion: one run retained a delete-pending name and another observed OS145 because the private directory was not yet empty. Treating only rejected OS32 as recoverable reports a post-publication error even when the exact owned stage is still reconcilable.

## What Changes

Reconcile typed uncertain child removal and verified delete-pending names within the existing two-second bounded cleanup window. Retry rejected removal only for native OS32; preserve all other rejected errors. Verify the original child and outer identities before each retry, allow only absence or pending names without mutation, and confirm outer disappearance before successful cleanup.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`kuru-memory` Windows private-stage cleanup and native-only fixtures. No public API, platform primitive, general deletion policy, publication retry, process lifecycle, or timeout configuration changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
