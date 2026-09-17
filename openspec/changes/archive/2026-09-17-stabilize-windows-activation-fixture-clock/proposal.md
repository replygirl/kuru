## Why

Windows native CI observed the positive checked-activation fixture return after zero retries because a synchronous native move and reconciliation consumed its real two-second deadline. The fixture is meant to prove one checked rejection followed by successful activation after releasing its held descendant, independent of runner wall-clock load.

## What Changes

- Add Tokio's existing test clock feature only as a `kuru-memory` dev dependency.
- In `packages/kuru-memory/src/provision/native_tests.rs`, pause test time only around the positive activation call, then resume before asserting its result; require one checked rejection and retain all identity, payload, cache-lock, and cleanup checks.
- In the persistent-blocker fixture, require at least one checked denial while retaining its real elapsed-budget and publication-stage assertions.

## Impact

Windows test timing only. Production deadline, retry spacing, native move behavior, and CI workload remain unchanged.
