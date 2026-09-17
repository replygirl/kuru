## Why

The Windows held-candidate-view fixture dropped a detached SQL connection and immediately retried abandonment; Dolt could still observe that session and correctly refuse the branch rename. The test must wait for that exact session to end before checking the successful second attempt.

## What Changes

- In `packages/kuru-memory/src/store/operational_gc_tests.rs`, capture the held connection ID with the existing helper and await its observed teardown after dropping it, before the second candidate abandonment.

## Impact

Test-only change. The first in-use refusal, branch/history assertions, production behavior, and existing bounded session teardown policy remain unchanged.
