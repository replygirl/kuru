## Why

The Unix built-in shell currently reaps or drops its root before a `Drop`
guard signals the saved numeric process-group ID. That ordering can target a
stale group, and timeout, output, read, caller-loss, and runtime-loss paths can
return without confirming root reap or group absence.

The shell already intends to terminate remaining members of its fresh group,
including after ordinary root exit. It needs an owner that preserves the root
identity, pipes, and retained workspace capability through bounded cleanup
without turning shell authority into a sandbox claim.

## What Changes

- Add a narrow safe Unix platform owner that spawns a fresh process group,
  retains its standard child, observes exit without reaping, consumes signal
  authority before reap, and permits only read-only absence checks afterward.
- Move built-in Unix shell capture and cleanup to a private connector-owned OS
  thread whose runtime survives caller-future and parent-runtime loss.
- Register active and unconfirmed shell owners with `ToolHost`; close
  registration and request/await their bounded cleanup during shutdown while
  retaining ownership after an unconfirmed result.
- Preserve the current command, environment, output, status, timeout,
  workspace-revalidation, MCP, and Windows Job behavior.
- Add native macOS and Linux process-group fixtures plus deterministic native
  state-transition tests; retain required Windows non-regression coverage.

## Capabilities

### New Capabilities

### Modified Capabilities

- `native-platform`: Add a small Unix fresh-process-group owner whose API
  prevents independent child reaping and post-reap numeric signalling.
- `provider-tools`: Make built-in Unix shell timeout, cancellation, capture,
  reap, group cleanup, and `ToolHost` shutdown ownership observable and bounded.

## Impact

Expected implementation is limited to a Unix process-group module/export and
fixtures in `packages/kuru-platform`, a private Unix shell-owner module plus
`ToolHost` integration and fixtures in `packages/kuru-connectors`, and accurate
an actual fake-provider CLI shell-turn fixture in `apps/kuru-tui`, and accurate
shell lifecycle wording in `AGENTS.md`, platform crate/module descriptions,
`docs/protocols.md`, and the curated tools reference.
The existing exact `rustix` and Tokio pins provide the needed APIs, so no
new dependency version or lockfile change is expected. The TUI fixture enables
the existing workspace `kuru-delivery` dependency's `tooling` feature only as a
dev-dependency to reuse its bounded subprocess runner; production connectors
do not depend on delivery.

## Surfaces

- [x] interactive — shell tool completion and cleanup failures are visible to
  terminal users
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — native Unix child, signal, wait, and process-group behavior
- [x] agent-behavior — built-in shell tool result timing and failure shape
