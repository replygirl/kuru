# Verification

## 1. A failed open's owner lock is free when the open returns [critical]

- [x] 1.1 @regression (agent) `service::activity::tests::a_failed_open_releases_its_owner_lock_despite_a_duplicate_descriptor`: a hook duplicates the owner lock's handle as a fork-to-exec child would; the open fails at endpoint publication; the test probes the lock once while the duplicate lives -> observed 2026-10-03 macOS arm64: before the fix FAILED `a failed open left its owner lock held by a duplicate descriptor` (0 passed, 1 failed); with the fix, `-- service::` 134 passed, 0 failed (105.3 s)

## 2. Designed behaviour unchanged

- [x] 2.1 @integration (agent) `mise run //packages/kuru-memory:test` (full) -> observed: exit 0, lib 698 passed, 0 failed, 6 ignored (713.0 s); bundle_build 10, memory 5, server_lifecycle 12, supervisor_snapshot 1 passed; wall 741 s
- [x] 2.2 @integration (agent) `mise run //packages/kuru-memory:test -- service::activity` five times -> observed: five runs, each `37 passed; 0 failed` (21.4-26.0 s), including the unchanged `a_failed_endpoint_publication_marks_then_retires_the_record`

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> observed: each exit 0 (docs tasks run with `NODE_OPTIONS` unset; the shell's preload breaks the docs toolchain); validate 0 errors, 0 warnings
- [~] 3.2 @runtime (agent) native memory partitions in PR CI on every supported platform -> defer: PR CI runs after the branch is pushed and a PR is opened; CI reruns are not used as evidence here
