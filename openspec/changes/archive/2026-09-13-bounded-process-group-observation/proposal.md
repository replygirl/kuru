## Why

The Unix post-cleanup observer for an owned process group currently treats an
`EPERM` result from `killpg(..., 0)` as an immediate cleanup failure. On macOS,
after Kuru has terminated its owned group and reaped the root, XNU can return
`EPERM` while zombie filtering leaves no signallable group member; a later
observation can report `ESRCH`. The immediate error makes bounded native
cleanup flaky even though this transient state is not proof that a process
remains alive. Apple's published [XNU signal implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c#L1612)
supports this possible transition; it does not establish that every observed
permission failure is transient.

`EPERM` must not be accepted as absence. A persistent permission result or any
other unexpected observation still needs to fail with a useful bounded
diagnostic, and cleanup must not issue another kill after the owned root has
already been reaped.

## What Changes

- Keep polling a post-termination `EPERM` only until the existing fixed
  five-second cleanup deadline; succeed only when the observer receives
  `ESRCH`.
- Preserve the existing `exists` to `ESRCH` success path and unexpected-error
  failure behavior.
- Add deterministic private observer tests for transient and persistent
  permission results, alongside the existing owned-descendant native fixture.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

<!-- None. The durable contract is unchanged; this corrects bounded native observation. -->

## Impact

The implementation will be limited to the delivery package's private Unix
process-group observation helper and its advisory fixture tests. It changes no
public API, generic output capture policy, MCP behavior, Windows behavior,
dependencies, or runtime configuration.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
