## Why

The compiler-free installer can hang while reading a missing local release asset on macOS. Its FIFO producer is started before the bounded reader, and an immediately failing producer can close during the FIFO open handoff without delivering a readable EOF to the later `head` process.

The hang leaves the existing installation unchanged, but reports a timeout instead of the real asset failure and blocks repository coverage. The retained producer and consumer identities still allow correct cleanup; their launch order must make the reader ready before an immediate producer exit.

## What Changes

- Start the bounded FIFO consumer before the release-asset producer.
- Retain both child identities and the existing size, status, signal, and cleanup checks.
- Verify normal, missing, corrupt, oversized, and held-open inputs preserve their current outcomes without a timeout.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None.

## Impact

The change is confined to `packages/kuru-delivery/support/install.sh`, its bootstrap-install regression tests, and this fix record. It does not change release archive names, limits, checksums, installation destinations, or the native updater API.

## Surfaces

- [x] interactive — corrects failure reporting and termination of the public shell installer
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
