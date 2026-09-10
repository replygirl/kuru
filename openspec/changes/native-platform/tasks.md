## 1. Independent package and filesystem

- [ ] 1.1 Create the platform library with package-owned mise tasks and target-specific pinned dependencies; verify package-only build and lint need no consumers/bundles and unsafe FFI remains confined to the Windows module.
- [ ] 1.2 Implement checked native directory/file identity, private root creation and inherited owner-only descendant validation; verify real Unix modes and Windows ACL/reparse/hardlink fixtures without modifying unsafe existing objects.
- [ ] 1.3 Implement retained-name policies and stable file locking with full identity revalidation; verify separate-process contention, pinned/movable handles and substitution attempts on native filesystems.
- [ ] 1.4 Implement explicit new-only/replace publication and uncertain-outcome context; verify real same-volume content/identity, occupied-target preservation, pre-move failure and controlled post-move reconciliation boundaries.

## 2. Windows processes and local IPC

- [ ] 2.1 Implement Windows NativeSpawnSpec with exact native argv/environment/stdio/handle intent; verify compiled fixture round trips and duplicate environment rejection without shell interpretation.
- [ ] 2.2 Implement atomic Job/handle-list process creation, cancellation and authoritative tree close/wait; verify owner-death startup phases, descendant lifetime, output draining and concurrent inheritance with native Windows fixtures.
- [ ] 2.3 Implement first-instance private overlapped local pipes and expected-peer checks; verify blocked connect/read/write cancellation, impostor rejection and complete Tokio shutdown on Windows without stranded workers.

## 3. Independent native acceptance

- [ ] 3.1 Add a required package-only windows-2025 CI job and aggregate dependency through owning mise tasks; verify actual native tests/coverage fail for missing prerequisites and do not compile application/database/bundle consumers.
- [ ] 3.2 Run cross-operation filesystem/process/IPC fixtures and Unix shared-filesystem checks at the candidate commit; record observed native evidence, preserve the workspace coverage floor and retain all unrun product Windows acceptance gates.
- [ ] 3.3 Document the safe API, native-only module boundaries, ownership/durability limits and package tasks; verify the dependency tree and guidance make no product Windows support claim or new Unix process-lifecycle promise.
