## Why

The current Dolt store reconciles a lost SQL reply with one receipt on the branch, but each subsequent write deletes that receipt. The local service adds another uncertainty boundary: a client can lose a write reply while the owner commits, and a sibling client can replace the only queryable receipt before the first client reconnects. Public managed writes need durable, request-bound outcome evidence before the service can safely admit later mutations or exact retries.

## What Changes

- Retain compact indexed receipts in the existing branch-local `operations` table, binding a stable logical mutation ID to its method, view and canonical request fingerprint without copying private payloads.
- Add a typed, bounded outcome query that distinguishes in-flight, committed, definitively absent and unresolved work. Keep a logical client fail-closed after an incomplete mutating exchange until its exact outcome is reconciled; never replay automatically.
- Upgrade current writable main and the permanent usage branch under checked lifecycle authority. Preserve pre-upgrade candidate branches and history unchanged as historical, stale views; newly writable candidates inherit the current receipt contract.
- Use existing candidate reference/base-target and usage invocation records for result-bearing reconciliation instead of treating every operation as a generic applied bit.

## Capabilities

### New Capabilities

- `durable-service-receipts`: request-bound retained operation evidence and typed outcome reconciliation across service attachments and owner restarts.

### Modified Capabilities

- `versioned-memory`: upgrade the current-schema receipt-retention rule while leaving prior branch schemas and candidate cleanup/GC requirements intact.

## Impact

`kuru-memory` owns the Dolt schema migration, branch validation, receipt writes and queries, and typed service/client operation IDs. `kuru-core`/runtime callers provide stable invocation or turn-operation identity where already available. Main and permanent usage branch receipts gain a versioned compact shape; a binary supporting only the older schema fails on newer writable memory. The public `MemoryStore` method names remain usable, while managed writes are held until this prerequisite and service facade integrate.

## Surfaces

- [ ] interactive — no new UI or command
- [x] deploy — service-owner restart and native crash recovery
- [x] integration — versioned Dolt branch and typed IPC contracts
- [ ] agent-behavior — no prompt or model behavior change
