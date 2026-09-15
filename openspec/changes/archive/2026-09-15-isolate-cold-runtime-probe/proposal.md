## Why

On Windows, a first launch from an empty managed-runtime cache can extract and exact-version-check the bundled Dolt executable successfully, then fail every checked attempt to rename that verified runtime directory into place with `ERROR_ACCESS_DENIED`. Native CI observed the held source unchanged and the destination absent across the full bounded recovery window, so increasing retries would preserve the failure without addressing the movable-directory contract.

The cold version probe currently executes the same file identity that activation immediately tries to move with its containing directory. Windows may still deny that directory move after the owned process tree reports completion, so the executable used for validation must not be inside the publication subtree.

## What Changes

- Copy the extracted cold executable through checked source and destination handles into a private probe-only sibling outside the runtime directory that will be activated.
- Verify the complete copied bytes, exact size, private executable policy, and held name/identity before running the unchanged exact-version probe.
- Retain the probe copy and its owned process through reap and keep the private staging owner through the checked runtime-directory move; probe cleanup cannot block publication.
- Keep warm-cache verification unchanged because it does not rename the verified installed directory.
- Add native regression coverage that proves the probe uses a distinct executable identity outside the activation source and that cold publication still rejects corrupt source or copy bytes before activation.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

This affects cold managed-runtime provisioning and native tests in `packages/kuru-memory/src/provision.rs`. It changes no cache layout, payload pin, persistent schema, public API, dependency, warm-open behavior, or publication validation.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
