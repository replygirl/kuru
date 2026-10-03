# Verification

## 1. A released key lock binds no duplicate of its description [critical]

- [x] 1.1 @regression (agent) `store::creation_template::tests::a_released_key_lock_is_free_while_a_duplicate_descriptor_remains`: take the key lock shared and exclusive, `try_clone` it as a fork-to-exec child would, release, take it exclusively while the duplicate lives -> observed 2026-10-03 macOS arm64: with `release_lock`'s unlock replaced by a no-op, FAILED `a released Shared key lock stayed held by a duplicate descriptor`; with the fix, passed (`-- duplicate`: 7 passed, 0 failed)
- [x] 1.2 @regression (agent) `store::creation_template::open_tests::an_inherited_duplicate_of_the_shared_key_lock_does_not_send_the_opener_cold`: a hook duplicates the opener's shared key lock and the test holds it through the open, with an injected copy read error -> observed: without the unlock, FAILED `a held duplicate of the released shared key lock sent the opener cold` (the unexpected store was closed, no unreaped-store report); with the fix, passed

## 2. Designed behaviour unchanged

- [x] 2.1 @integration (agent) `mise run //packages/kuru-memory:test` (full) -> observed: exit 0, lib 696 passed, 0 failed, 6 ignored (706.85 s); bundle_build 10, memory 5, server_lifecycle 12, supervisor_snapshot 1 passed; wall 775.7 s
- [x] 2.2 @integration (agent) `mise run //packages/kuru-memory:test -- store::creation_template` five times -> observed: five runs, each `53 passed; 0 failed` (97.9-105.1 s), exit 0

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> observed: format:check, lint, lint:windows, typecheck and docs:check each exit 0; `cospec validate --all --strict` 0 errors, 0 warnings
- [~] 3.2 @runtime (agent) native memory partitions in PR CI on every supported platform -> defer: PR CI runs after the branch is pushed and a PR is opened; CI reruns are not used as evidence here
