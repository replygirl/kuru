## Why

The Windows warm-cache corruption fixture can receive an uncertain access-denied result while marking the recently probed executable for deletion. Its test-only invalidation helper currently recovers only sharing violation 32, so the fixture fails before checking that a corrupt cache is rejected.

## What Changes

- Extend the fixture helper's existing bounded, identity-checked retry to an uncertain native access-denied result, while retaining the prior sharing-violation case.
- Keep the real warm-cache concurrency, payload verification, and corrupt-cache rejection assertions.

## Impact

Only `packages/kuru-memory/src/provision/native_tests.rs` changes. The Windows native fixture may use its existing two-second recovery budget; production deletion and provisioning remain unchanged.
