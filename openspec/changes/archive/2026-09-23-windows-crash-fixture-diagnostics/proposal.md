## Why

The native Windows crash-recovery fixture can time out while its retained service child is still running, but its current failure merges a missing endpoint with an unusable published pipe and discards the child and Dolt startup diagnostics needed to identify the cause.

## What Changes

- Distinguish the fixture's last observed missing-endpoint and published-transport states without changing its deadline or retry behavior.
- Retain bounded child stderr and the existing bounded private Dolt startup log in the terminal failure.
- Add source-level tests for the bounded diagnostic formatting where practical.

## Impact

Only `kuru-memory` test support and its Cospec verification record change. Product service startup, election, transport, and timeout behavior remain unchanged; CI time is materially unchanged.
