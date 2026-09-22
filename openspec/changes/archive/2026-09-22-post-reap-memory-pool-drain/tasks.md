## 1. Reproduce and correct shutdown

- [x] 1.1 Add a real-Dolt held-pool regression that fails against the original close behavior and proves owner reap before releasing the dead client connection.
- [x] 1.2 Give the same closed pool handles one separately bounded post-reap drain in both owner shutdown paths; preserve lifecycle authority through exact child reap and report failure if post-reap pool cleanup remains unproved.

## 2. Verify and deliver

- [x] 2.1 Run the focused regression before/after, existing disconnect fixture, and affected package checks; record exact observed evidence.
- [x] 2.2 Obtain independent source review and strict validation before archiving and committing the isolated fix for normal delivery and native CI.
