## Why

Every managed warm memory open currently holds the exclusive Dolt installation lock while it fully digests the cached engine and licenses and runs the exact-version probe. The digest is required, but serializing independent warm verification is not; additionally, owner-controlled Unix data directories rejected for broad permissions receive an actionable mode-0700 remedy only when a legacy SQLite file is present.

## What Changes

- Verify an existing managed Dolt cache concurrently through retained checked handles, with full payload digests and name/identity revalidation before the pathname-based version probe.
- Reserve the exclusive installation lock for an absent cache, recheck the destination under that lock, and retain existing staged extraction, publication, and uncertain Windows activation guarantees.
- Give any rejected current-user-owned Unix data directory with group or other permission bits an exact-directory mode-0700 remedy, whether or not legacy SQLite exists, without changing permissions or advising on links and foreign-owned paths.
- Measure and document the honest warm-open result without introducing a metadata shortcut, daemon, or weaker integrity check.

## Capabilities

### New Capabilities

### Modified Capabilities

- `embedded-runtime`: Existing cached runtimes verify concurrently while cache installation and publication remain serialized.
- `versioned-memory`: Managed warm-open concurrency and owner-owned Unix data-directory remediation apply to ordinary and legacy memory layouts.

## Impact

This affects managed-runtime provisioning and its native tests in `packages/kuru-memory`, the shared data-directory remediation helper in `packages/kuru-memory/src/migration.rs`, focused CLI wiring owned by the coordinating change, and memory/startup documentation. It changes no persisted schema, cache payload, digest, version probe, automatic permission behavior, or public dependency.

## Surfaces

- [x] interactive — rejected owner-owned Unix data directories receive actionable user-facing guidance
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
