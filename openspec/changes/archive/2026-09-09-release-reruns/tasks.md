## 1. Reproduce and repair recovery

- [x] 1.1 Add and run a regression that fails when an already-published matching release is retried.
- [x] 1.2 Implement strategy-only planning and exact existing-bump reconciliation with real Git/HTTP interruption and conflict tests.
- [x] 1.3 Make published release recovery and Actions artifacts idempotent while retaining immutable publication checks.

## 2. Integrate and verify

- [x] 2.1 Update release instructions and agent guidance; preserve inline final Pages jobs.
- [x] 2.2 Run the full mise gate, record observed evidence, and validate the change for archive before committing.
- [x] 2.3 Review dispatch and recovery against cospec and GitHub's rerun contract; prepare the change for required PR checks without dispatching a release.
