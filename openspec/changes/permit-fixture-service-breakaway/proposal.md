## Why

Windows application fixtures launch each checked command in a nonpermitting owned Job and wait for that complete Job to quiesce. After the contained-owner fallback was added, a Kuru command can finish successfully while its intentionally warm memory owner remains in that per-command Job, so the fixture reports a process-tree timeout before its authenticated service cleanup can run.

The ordinary ownership boundary is correct for arbitrary commands. Kuru fixtures need an explicit launch intent that permits only a descendant which requests `CREATE_BREAKAWAY_FROM_JOB` to leave the immediate command Job, while a nonpermitting runner Job continues to contain it and fixture cleanup retires it.

## What Changes

- Add an explicit Windows fixture command option which selects the existing breakaway-permitting, kill-on-close Job.
- Use that option only at application fixture launches that can start a managed memory service, including the ConPTY child launch.
- Retain authenticated managed-service cleanup for each fixture data root before its temporary files are removed.
- Keep ordinary command/process ownership, product service fallback, idle timeout, and production limits unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

The test/maintainer command facade gains a fixture-only launch selector on Windows. Application integration fixtures opt into it and add missing service retirement at their existing ownership boundaries. No shipping CLI, memory protocol, service election, or production process lifetime changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
