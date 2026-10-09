# Proposal

## Why

Darwin's Mio Unix listener creation sets close-on-exec after creating the socket,
but Kuru's private service socket creation does not coordinate with owned child
creation. A concurrent owned child can inherit the transient descriptor and
keep a dropped listener alive; native CI observed a successful connection to a
fixture's supposedly closed listener, though its historical inheritor is unknown.

## What Changes

- Hold the existing platform spawn lock across synchronous Unix listener binding,
  outgoing socket creation, and each poll of the existing accept future; release
  it before every asynchronous wait.
- Route the private service listener and its native endpoint fixtures through
  that internal binding operation.
- Prove lock admission causally and require a dropped listener to refuse a
  connection while an actual owned child remains alive.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing ownership and private IPC requirements remain correct.

## Impact

Only the platform's private Unix socket creation, internal spawn-lock visibility,
associated native regression tests, and contributor ownership documentation
change. No external API, dependency pin, new lock, deadline, or permission
policy is introduced.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
