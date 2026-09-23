## Why

On Windows, an elevated process can have a distinct `TokenOwner` and `TokenUser`. The staged-file owner handoff then exposes two defects: the create-time zero-rights `OWNER RIGHTS` ACE is not guaranteed to remain effective after changing the stage to `TokenOwner`, and the native ACL fixture reopens its retained object without the read-control authority needed by its checked DACL mutation.

Hosted Windows coverage consequently rejects valid staged publication before the source DACL can be copied and prevents the strict weak/null-DACL fixtures from reaching their assertions. The correction must retain exact-handle ownership, keep the stage private throughout the handoff, and leave production source handles read-only.

## What Changes

- Reapply the existing protected private file DACL through the retained staged handle before changing a stage to a distinct supported token owner, then preserve the existing post-assignment privacy and exact-owner checks.
- Give the test-only exact-object DACL mutation handle the complete `READ_CONTROL | WRITE_DAC` authority required by its checked fixture operation.
- Cover distinct `TokenOwner` handoff without weakening broad/null ACL rejection or publication fencing.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

The implementation is confined to `packages/kuru-platform/src/windows/security.rs` and Windows-native platform fixtures. It changes no public API, publication policy, source-file authority, dependency, migration, or non-Windows behavior.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
