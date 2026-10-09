# Proposal

## Why

The native Windows platform suite passes, but its 95% line gate still exposes untested ownership and refusal contracts. Exercise real retained objects and peer exit, and simplify redundant test-only error translation while preserving all native observations and cleanup.

## What Changes

- Extend Windows filesystem and security tests with wrong-object/type rejection and real insufficient-access refusal; assert exact identities, ACLs and bytes remain unchanged.
- Extend existing native fixtures with pending-pipe error propagation, live and reaped trusted-child diagnostics, read-only current-image access, and command/environment validation before execution.
- Replace console test-helper mismatch errors with exact observed-state assertions after native restoration; retain native API errors and checked focus-record decoding.

## Impact

Only `packages/kuru-platform` tests and test-support helpers change. Native Windows CI provides the behavior and coverage evidence; the 95% gate, pins, production interfaces and support claims remain unchanged.
