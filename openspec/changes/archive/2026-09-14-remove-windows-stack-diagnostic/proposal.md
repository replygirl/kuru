## Why

The temporary CDB stack-text diagnostic has produced its native evidence and
the stock-shell bootstrap now passes its Windows fixture. Keeping the debugger
download, workflow setup, and capture-only test code would add permanent CI
cost and maintenance without serving the shipped behavior.

## What Changes

- Remove the CDB-only capture helpers, selector, metadata, extended diagnostic
  watchdog, and known-sleep control from the connector test fixture.
- Remove the Windows SDK debugger preparation script, package task, workflow
  setup, and debugger-only workflow assertions.
- Preserve the stock-module bootstrap, RPC readiness regression, ordinary
  Windows shell fixtures, sharded coverage receipts, and fail-closed aggregate.

## Impact

This touches test-only code in `packages/kuru-connectors/src/tools.rs`, delivery
debugger support and task wiring, the Windows native workflow, and its focused
workflow validator. Production shell behavior and normal test deadlines do not
change; the cleaned source still requires the ordinary exact-head CI matrix
before merge.
