## Why

The P14 lifecycle-hook Windows source installation fails to compile at the
configured hook command boundary. The private connector helper accepts a generic
environment iterator, but the native Windows process API requires the owned
vector that it validates and stores in the child specification; the only caller
already has that vector. A Unix-only RPC standard-I/O import also appears as an
unused Windows warning.

## What Changes

- Match the private connector helper's environment parameter to the existing
  owned Windows process API and its sole caller.
- Gate the RPC standard-I/O import to Unix, where it is used.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The lifecycle-hook and owned Windows process contracts are already
correct; this fixes their implementation boundary.

## Impact

Only `packages/kuru-connectors/src/process.rs` and `rpc.rs` change. There is no
wire, configuration, dependency, release-format, or platform API change.

## Surfaces

- [ ] interactive — no command or UI behavior changes.
- [ ] deploy — no deployment or runtime topology changes.
- [ ] integration — no external protocol or schema changes.
- [ ] agent-behavior — no hook authority, prompt, or tool-output change.
