# Proposal

## Why

Two CI failure signatures in the memory service start path carry too little
information to diagnose. `memory service readiness deadline exceeded` is raised by
`attach_or_start` after one flat `startup_timeout_secs` covers election, the owner
probe, the owner spawn and the owner's whole open, but the error does not say how
that time was split, so a client that waited behind a closing owner looks the same
as an owner that opened slowly. `contained memory starter exited before readiness:
exit code: 1` comes from the Windows-only contained starter fixture, which launches
its child with the platform's default null stderr, so the child's actual error is
discarded and exit code 1 matches any failure (seen once, main run 36601116359).

## What Changes

- The readiness deadline error in `attach_or_start` keeps its existing text,
  including the test-support startup observations, and appends the client-side
  phase split it measures with the `tokio::time::Instant` clock already used for
  the deadline: `client phases: election=<ms>; owner-probe=<ms>; spawn=<ms>;
  readiness=<ms>; polls=<n>; child=running; last-attach=<outcome>`. The
  millisecond fields are consecutive offsets from one start instant, so they sum
  to the whole elapsed wait. `last-attach` reports what the last readiness poll
  already observed (`no-endpoint`, `transport-unavailable` or `peer-closed`),
  through a pure refactor of `try_attach` that adds no IO. No deadline, poll
  interval, sleep, retry or success-path IO changes; the election and owner-probe
  deadline errors are unchanged.
- The Windows contained starter fixture gives its child a private stderr file.
  Its two readiness waits share one helper with the same 20 ms poll and
  deadlines; when the child exits early or misses its deadline, the failure
  appends the bounded 4 KiB tail of that file. A file never back-pressures the
  child, so no draining is needed while it runs.
- User documentation for the memory startup timeout states what the deadline
  error reports.
- Test-only: a Unix test drives a real owner launch that stays alive without
  publishing an endpoint under the existing lowered 1 s startup bound; a Unix
  test and a new Windows test prove the starter wait surfaces the child's stderr.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-memory/src/service.rs`: `attach_or_start` readiness error text
  (appended only), an internal `try_attach_observed`/`AttachOutcome` split of
  `try_attach`, and test-only fixture and test changes.
- `docs/configuration.md`, `apps/kuru-docs/reference/configuration.md`: one
  sentence on the deadline error.
- The only user-visible effect is appended text on an existing error. No
  interactive, terminal or command behaviour changes.
- No protocol, configuration, dependency, lockfile or workflow change. The
  owner still reports no per-stage progress or timestamps to the client in
  product builds; that needs a new owner-to-client channel and is out of scope.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
