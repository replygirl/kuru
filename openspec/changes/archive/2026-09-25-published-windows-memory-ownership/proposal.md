## Why

The published Windows download verifier runs Kuru inside a Job that keeps its independent memory service in the command tree. A cold demo conversation can complete and produce valid JSON, but the verifier then rejects the still-live service as a command that did not settle. The v0.8.0 post-public verification failed twice: the second attempt reached `Memory: ready` and a completed response before this process-tree refusal. The first attempt also had a memory readiness timeout; its precise startup delay is not established by the available log.

## What Changes

- Permit only the memory service's explicit independent-service breakaway from the verifier-owned mise command Job, as the already-passing staged Windows fixture does.
- Retire the isolated project's service with the installed Kuru's existing `memory purge --yes` command after the cold conversation, reopen, and bundled-engine checks. This command targets only the verifier-created temporary project and data root; it obtains the memory service maintenance permit before removing that disposable data. Preserve the isolated root and report cleanup uncertainty on any failed verification.
- Attach a fixed command-phase name to bounded native command failures so a future failure identifies the exact stage without exposing paths or private output.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

The delivery package's published Windows verification helper and focused native tests change. The public Kuru runtime, bundled assets, release workflow, checksum policy, production memory service and bundle-preparation dependency graph do not change. The already published v0.8.0 tag and assets remain immutable; this correction is for a later reviewed release.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
