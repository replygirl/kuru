## 1. Native service foundation

- [x] 1.1 Use the archived platform IPC primitives to add bounded framing and generation-bound handshake in `kuru-memory`, and verify wrong identity, secret, version and oversized frames before storage access. Completed by archived `project-memory-owner-foundation`.
- [x] 1.2 Add service election, bootstrap and internal executable entry with a retained stable lock, and verify barrier-synchronized cold starts produce one owner and one real Dolt engine. Completed by archived `project-memory-owner-foundation`.
- [x] 1.3 Keep the archived service-owned local `MemoryStore`, branch pools and supervisor; add a native fixture that exits the starter process while a surviving client stays attached and completes read/write without engine teardown. Completed in archived `independent-windows-memory-owner`; hosted native Windows execution remains pending exact-head CI.

## 2. Client and lifecycle integration

- [x] 2.1 Add a typed service client facade for used store, candidate and usage-ledger operations with bounded request IDs and no raw SQL/credential exposure, and verify existing memory integration fixtures through the facade.
- [x] 2.2 Move client close/drop to attachment release and add operation permits, idle timer and owned shutdown order, and verify disconnect, accepted-work drain and reconnect-at-expiry races.
- [x] 2.3 After `durable-service-operation-receipts` is archived and integrated, attach normal CLI runtime writable opens to the service while retaining the conversation-driver lease, and verify sequential `kuru run` processes reuse one warm engine without simultaneous conversation admission or unresolved write outcomes.
- [x] 2.4 Route inspection and maintenance entrypoints through the owner or explicit quiescence gates, and verify migration, candidate, ledger, forget and purge behavior against actual Dolt.
- [x] 2.5 Add bounded exact-ref candidate inventory/status and checked explicit abandon through the existing memory CLI and shared TUI registry. Verify retained refs remain discoverable after client/owner restart; changed, active, historical-stale or uncertain refs refuse without deleting private history, and no public promote or automatic replay appears.

## 3. Recovery and compatibility

- [x] 3.1 Recover stale endpoint records only after verified election/lifecycle authority; use the prerequisite's request-bound typed outcome reconciliation before subsequent managed mutation, and verify service/Dolt crash fixtures never duplicate effects or kill an unrelated process.
- [x] 3.2 Reject incompatible active service and schema versions with actionable CLI diagnostics, and verify old/new fixture handshakes leave the active owner untouched.
- [x] 3.3 Update AGENTS.md and owning memory/development/installation docs for the service lifetime, privacy, idle interval and transitional conversation lease, and verify docs build and link checks.

## 4. Cross-surface acceptance

- [x] 4.1 Record observed evidence for every verification-ledger row, including native Windows/macOS/Linux memory and packaged offline install/update checks; name any genuinely unrun check and its reason. Reconciled 2026-09-22: local and inherited observed evidence is marked complete; exact-P27-head hosted permission, packaged install/update, and normal combined-coverage gates are explicitly deferred in the ledger.
- [ ] 4.2 Run relevant package checks, combined coverage and strict Cospec validation, resolve failures, then archive the change and verify its archive exists before the final branch commit.
