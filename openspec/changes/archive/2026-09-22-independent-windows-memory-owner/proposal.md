## Why

The new Windows memory service is launched with a detached process handle, but that does not remove a child from a Job already containing its starter. If that Job has kill-on-close behavior, the service and its Dolt owner can die when the starter exits even while another client remains attached. The service has not been published yet, so this ownership gap must be corrected before its first PR.

## What Changes

- Add a service-specific native launch lifetime that requests Windows Job breakaway while retaining the child process handle for readiness and observation. If the containing Job forbids breakaway, startup fails with a precise diagnostic instead of silently promising independent lifetime.
- Prove on native Windows that an allowed-breakaway starter can exit while an independently attached client continues to read and write, and that a Job denying breakaway rejects launch without publishing an owner.
- Preserve existing trusted-supervisor and owned-job launch behavior for Dolt, browser and other consumers.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None; the archived `project-memory-owner` specification already requires the service to outlive its starter.

## Impact

- `packages/kuru-platform/src/windows/process.rs` gains one narrow safe lifetime selection and native Job fixtures.
- `packages/kuru-memory/src/service.rs` uses it only for Windows service spawn and adds the starter-exit/survivor fixture.
- No data schema, public control API, credential flow or provider path changes.

## Surfaces

- [ ] interactive — no user-facing command change; startup already reports errors
- [x] deploy — Windows process containment affects packaged service lifetime
- [x] integration — Windows Job and service IPC process behavior
- [ ] agent-behavior — no model, tool or prompt change
