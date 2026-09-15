## 1. Bounded retained ownership

- [x] 1.1 Add process-wide fixed retained-shell admission whose permit follows the real worker/group and releases only after confirmed cleanup or prelaunch failure.
- [x] 1.2 Change indefinite retained cleanup to capped exponential single-observation retries while retaining existing phase-safe signal-before-reap behavior.

## 2. Verification

- [x] 2.1 Add deterministic real-owned shell tests for cross-registry admission, overload-before-spawn, retained dropped-owner capacity, exactly-once later release, prelaunch release, and bounded retry observations.
- [x] 2.2 Run the granular Unix shell test target and strict Cospec validation; record only observed evidence.
