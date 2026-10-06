# Proposal

## Why

Native Windows CI disproved the held-root deletion-pending premise in the prior test record: the name was already absent while its handle remained open. The historical retained-supervisor OS5 failure still lacks operation attribution and needs bounded observations of its actual fixture path.

## What Changes

- Restore immediate contextual error propagation in the existing owned-removal observer and remove the disproved held-root case together; preserve precise handoff, pool and removal contexts.
- Run the original retained-supervisor lifecycle case eight isolated sequential iterations on Windows, one elsewhere, with iteration and operation context. Stop on the first error; retain original ownership, cleanup assertions and each existing removal budget.

## Impact

Only `packages/kuru-memory/tests/server_lifecycle.rs` and this typed test record change. No production, interop, dependency, timeout or deletion-retry change. Windows performs at most eight original fixture openings; native CI is required for phase attribution. This change does not claim resolution of the historical error; PR auto-merge stays off pending evidence and final archival.
