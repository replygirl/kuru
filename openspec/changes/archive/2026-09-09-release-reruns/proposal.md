## Why

Release dispatch requires maintainers to copy a version and commit SHA to recover an interrupted run, unlike cospec's strategy-only interface. Rerunning a bump after GitHub accepted its commit but before the runner recorded its output currently fails because main has already moved.

## What Changes

- Keep only the auto/major/minor/patch strategy input and infer an existing release identity from immutable source history.
- Recognize an already-created version commit by its parent, message and complete stamped tree; reuse it without another write.
- Make publication of an already-complete matching release a verified read-only success and make workflow artifact uploads retryable.
- Document ordinary job reruns, preserve release-owned final Pages jobs, and cover interruption/conflict behavior with real Git and HTTP fixtures.

## Capabilities

### Modified Capabilities

- `release-automation`: replace explicit resume bookkeeping with automatic immutable recovery.

## Impact

Release workflow, package-owned Rust release CLI/library/tests, release operations and canonical agent guidance. No new dependencies, credentials, app behavior or independent deployment entrypoints.

## Surfaces

- [x] interactive — workflow dispatch and release CLI
- [x] deploy — release jobs and artifact recovery
- [x] integration — GitHub commit, tag, release and Actions rerun contracts
- [ ] agent-behavior
