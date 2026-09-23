## Why

Windows `IndependentService` always requests `CREATE_BREAKAWAY_FROM_JOB`. When Kuru runs inside a legitimate outer Job that forbids breakaway, `CreateProcessW` fails with access denied before the project memory owner starts, so ordinary application and packaged offline flows cannot open memory.

The service must remain independent of its starter process while respecting outer host or runner containment. It must still break away when permitted, and containment must never weaken owner locks, Dolt cleanup, private IPC, or crash recovery.

## What Changes

- Keep the independent-service breakaway launch as the first attempt.
- If and only if that exact create call fails with `ERROR_ACCESS_DENIED` and the current process is confirmed to belong to a Job, recreate mutable launch state and retry once without the breakaway flag.
- Keep the contained service independent of the starter process handle while explicitly retaining the inherited nonpermitting Job lifetime.
- Prove permitted breakaway, contained starter exit, outer-Job termination of the complete owner/Dolt tree, and clean successor recovery.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `project-memory-owner`: clarify Windows starter independence when a legitimate outer Job forbids breakaway, including outer-containment termination and ordinary recovery.

## Impact

- `packages/kuru-platform/src/windows/process.rs` gains one narrowly classified create fallback and native Job regressions.
- `packages/kuru-memory/src/service.rs` updates the Windows launch diagnostic and native service lifetime fixture.
- The process API, memory protocol, schema, user flow, dependencies, and non-Windows behavior remain unchanged.

## Surfaces

- [ ] interactive — no command or user-flow change
- [x] deploy — Windows host and CI Job containment affects service startup and lifetime
- [x] integration — Windows process Job semantics and private service lifecycle
- [ ] agent-behavior — no provider, tool, prompt, or model behavior
