## 1. Regression and correction

- [x] 1.1 Add a deterministic real-checkpoint-lease test with a cloned lock handle, and record its focused native red result before the fix: active owner excludes a second lease, but post-Drop reacquisition fails while the clone survives.
- [x] 1.2 Explicitly release the held advisory lock when `CheckpointLease` ends, and verify the same focused test passes without retry while active-owner exclusion remains intact.

## 2. Scoped verification and delivery

- [x] 2.1 Run owning connector all-target typecheck/lint and the affected file-checkpoint fixtures, and record observed outcomes without treating them as proof of the exact historical CI timing.
- [x] 2.2 Run strict Cospec validation/apply and record normal branch hooks plus native supported-platform CI as explicit post-archive publication gates, without claiming them complete before the fix can be committed and pushed.
