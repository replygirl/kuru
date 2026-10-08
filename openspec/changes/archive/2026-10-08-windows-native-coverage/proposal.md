# Proposal

## Why

Native Windows platform behavior passes, but its unchanged 95% coverage gate
still fails. The actual report identifies unexercised console, pipe lifetime,
file deletion and access-policy contracts worth testing directly.

## What Changes

- Exercise public console capture, VT admission and focus records through
  explicit inherited console handles in the existing isolated console fixture.
- Add real peer-death and pending-I/O Drop/runtime-shutdown cases using the
  existing bounded named-pipe/process fixtures; test batch representation and
  working-directory refusal before a marker executes.
- Cover native deletion blockers, READONLY/pending removal, stale retained
  directory authority and NULL/empty/unprotected/changed DACL boundaries using
  existing isolated filesystem and security fixtures.

## Impact

Only platform tests and their existing feature-gated fixture helpers change.
Production policy, unsafe-code boundaries, native wait budgets, dependency/tool
pins, source inventory and the canonical 95% coverage metric remain unchanged.
