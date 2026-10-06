# Proposal

## Why

Default production builds fail with E0425/E0433 because AttachmentIdentity uses Arc while its existing import is gated to tests or test-support. All-feature checks include that import and masked the production compile failure observed by native build and installation CI.

## What Changes

Make the existing std::sync::Arc import unconditional. Preserve all attachment identity, session proof and runtime behavior.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

One import in packages/kuru-memory/src/service.rs and this fix record only. No API, behavior, dependency, configuration or durable specification change.

## Surfaces

- [ ] interactive — no UI change
- [ ] deploy — no execution topology change; production compilation only
- [ ] integration — no external contract change
- [ ] agent-behavior — no runtime behavior change
