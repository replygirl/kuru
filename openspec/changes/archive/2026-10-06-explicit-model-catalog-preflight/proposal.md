# Proposal

## Why

An explicitly selected provider and model still make `kuru models` inspect or
open project memory before requesting the provider catalog. A legacy memory
sentinel can trigger migration even though catalog inspection needs only the
selected provider route and Kuru-owned authentication.

## What Changes

When both `--provider` and `--model` are supplied to `models`, resolve configuration
without saved preferences and return the existing provider catalog before memory
scope, lease, opening or migration. Review only relevant provider-route authority
for that invocation. Preserve the existing saved-selection behavior and memory
authority review for default `models` invocations.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Owning CLI dispatch, CLI behavior tests and `docs/configuration.md`. No provider
transport, credential storage, default invocation behavior or dependency changes.

## Surfaces

- [x] interactive — CLI catalog inspection
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
