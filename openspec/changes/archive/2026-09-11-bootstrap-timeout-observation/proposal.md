## Why

The e7eb136 macOS CI run timed out in the shell installer's host-detection test,
but its fixture discarded partial output and did not name the simulated host.
The fixture's consuming wait could also reap the root before its process-group
cleanup guard ran, leaving the timeout unable to distinguish a running installer
from an exited root with a descendant holding an output pipe. These are observed
diagnostic and ownership defects; the cause of the original installer stall is
not established.

## What Changes

- Retain bounded subprocess output, EOF observations and explicit case labels
  across the fixture's existing 30-second timeout.
- Keep the fixture's owned root unreaped while process-group termination authority
  is needed, then await cleanup before reporting the failure.
- Exercise actual controlled root/descendant stalls and retain known private-stage
  metadata so a recurrence identifies the reached state.
- Preserve production shell behavior, installation checks and existing deadlines.

## Capabilities

### Modified Capabilities

None. Existing delivery and verification contracts are correct.

## Impact

The change belongs to `packages/kuru-delivery/tests/bootstrap_install.rs` and a
small owning test-support module if needed. Use the already-pinned safe rustix
process API as a Unix test dependency; root owns Cargo manifests and lockfiles.
No application command, installer script, release graph, provider, external
service or credential is changed.

## Surfaces

- [ ] interactive — no product interaction change
- [ ] deploy — existing native CI topology remains unchanged
- [x] integration — real Unix child status, pipes and process-group ownership
- [ ] agent-behavior — no model or tool authority change
