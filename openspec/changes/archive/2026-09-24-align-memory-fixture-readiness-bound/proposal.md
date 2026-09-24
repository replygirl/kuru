## Why

Two Windows memory lifecycle fixtures use 20- and 40-second readiness waits even though their real owned startup can consume the configured 30 seconds before the tested action, and each fixture already has a longer overall bound. A missing readiness marker at those shorter deadlines does not prove the lifecycle contract failed.

## What Changes

- In `packages/kuru-memory/src/service.rs`, make each initial readiness wait use its existing 120- or 110-second absolute fixture deadline while retaining process-exit, status, receipt, containment, and cleanup assertions.
- Keep the observed startup phase unknown where the current child output does not identify it; add no product retry, startup allowance, or service topology.

## Impact

Only the two native memory fixture waits change. Their overall test budgets and production behavior remain unchanged; a genuinely stalled owner still fails within the existing bound.
