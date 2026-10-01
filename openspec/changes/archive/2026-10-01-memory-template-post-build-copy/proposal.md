# Proposal

## Why

The archived `memory-template-creation` change merged a "Store creation path
selection" requirement whose cold-fallback clause says any copy that fails
with a verdict against the template's bytes or an I/O error sends the opener
to the cold staged build. That clause has no exception for the copy a first
launch makes from the template its own open just built. The code (and the
regression test `failed_copy_after_the_build_fails_the_open_without_a_cold_retry`)
fails that open instead, because a cold retry there would run the schema
chain a second time inside one startup deadline, the outcome design 2.3
rejected. The living requirement and the shipped behaviour therefore
contradict each other. A later change audited against the spec could
reintroduce the five-start retry, or a reviewer could pass the
"sends the opener cold" scenario against code that fails the open.

The same requirement says the project's startup lock returns only after
every engine the creation worker started has been reaped. For a failed
template build engine whose supervisor overruns the reap allowance, the key
lock's guard passes to the background reaper and can outlive the startup
lock. That engine runs on the template build store, never on the project's
stage, and the key lock is its reap guard. The sentence over-promises.

The user-facing error also misnames the phase: every failure in the
build-then-copy path is wrapped as a template build failure, so a published
template followed by a failed copy reads as "build the memory store template:
copy the new project from the store template this open built: ...".

## What Changes

- Restrict the cold-fallback clause, and the WHEN of its scenario, to a
  template this open did not build.
- Add the post-build copy (from the template the open published, or from the
  build's verified stage) to the fail-the-open list. Say what it leaves:
  a preserved unstarted remnant, a quarantine only on a verdict against a
  template this open published, and an untouched template on an I/O error.
  Add a regression scenario for it.
- Narrow the startup-lock sentence to engines started on the project's stage.
  The template build engine's reap is guarded by the key lock, which may
  outlive the startup lock and which no product opener waits on.
- Give the post-build copy failure its own error variant, so the open's error
  says the copy from the template this open built failed and does not say the
  build failed. The no-cold-retry and quarantine behaviour is unchanged.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `versioned-memory`: the "Store creation path selection" requirement's cold
  clause, fail-the-open list and startup-lock sentence, plus one new
  regression scenario.

## Impact

- `packages/kuru-memory/src/store/creation_template.rs`: `CreateError` gains
  a variant for the copy that follows this call's own build. `create_in`
  returns it, and `Display` names the phase.
- `packages/kuru-memory/src/store/creation_worker.rs`: the worker gives that
  variant its own error context. It still fails the open with no cold retry.
- `packages/kuru-memory/src/store/creation_template/open_tests.rs`: the
  regression test asserts the error names the copy and does not name a build
  failure.
- `openspec/specs/versioned-memory/spec.md` through this change's delta.
- No change to start counts, locks, quarantine, preservation, timeouts or the
  activity sentence. The only user-visible difference is the text of the
  error a failed first-launch open reports. No terminal flow, layout or
  sentence changes, so the interactive surface is not checked.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
