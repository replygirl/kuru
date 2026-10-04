# Proposal

## Why

On PR #209 run 37179074716 (ubuntu-latest coverage partition 5, job 111367908240)
`real_pty_commands_complete_and_clear_only_the_visible_conversation` in
`apps/kuru-tui/tests/terminal.rs` failed. The managed-memory fixture printed the
owner's stderr (`<root>/private/owner-diagnostic.log`): "memory server startup
failed: Dolt startup/lifetime failed; private diagnostics:
<root>/private/data/memory/<hash>.staging-<uuid>/server.log: Dolt exited before
readiness (exit status: 1)". Dolt's own reason is only in that `server.log`, which
neither the failure text nor the CI diagnostics artifact carries, so the cause of
this failure family is unknown.

## What Changes

- `ServiceCleanup::owner_diagnostic` in `apps/kuru-tui/tests/support/memory.rs`
  (the fixture shared by every kuru-tui integration binary through
  `#[path = "support/memory.rs"]`): when the owner diagnostic it reports names
  `private diagnostics: <path>` and that path is strictly inside the fixture's own
  temporary root, it appends a bounded tail of that file, labelled with the path.
- A named path outside the root is not read, and the output says so. A missing or
  unreadable file is reported as such; nothing in the diagnostic path panics.
- The tail bound reuses an existing repository constant for diagnostic/log tails
  (candidates: `STARTUP_TAIL_BYTES` in `packages/kuru-memory/src/test_support.rs`,
  `FIXTURE_DIAGNOSTIC_TAIL_BYTES` in `packages/kuru-memory/src/service.rs` tests,
  both 4 KiB, over a `server.log` the owner already bounds with `LOG_LIMIT` of
  32 KiB in `server.rs`); its derivation is cited at the use site. No new literal.
- A test pins: tail appended for an in-root file, bound respected, out-of-root path
  not read, missing file reported.
- Non-goals: no product change, no change to what the owner writes, no change to
  timeouts or readiness, no CI workflow change, no retry.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `apps/kuru-tui/tests/support/memory.rs` and a test of it; if the bound constant
  must be shared, the smallest visibility change to expose the existing constant,
  not a copy.
- Test support only; no runtime, documentation or dependency change. Adds a few
  KiB to a failing fixture's output only when Dolt exits before readiness.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
