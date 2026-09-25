## Context

Schema v5 stores context summaries and cursors atomically, but its context record requires a turn string. Private reasoning summaries have a separate receipt-bearing batch mutation whose identity also requires a turn. An internal compaction has a real operation and invocation but may have no conversation turn; performing the private batch separately can invalidate the revision captured for the context checkpoint.

The branch already contains the archived session/source provenance and cursor-window storage contracts, plus the concrete local/remote memory facade and logical-receipt transport. The usage ledger deliberately remains on its own v4 registry.

## Goals / Non-Goals

**Goals:**

- Represent speaking-turn and internal-operation provenance without relabeling historical data.
- Publish the context summary, cursor and accepted private sidecar in one checked transaction and one receipt outcome.
- Reject malformed or oversized combined requests before a mutating attachment and again at the owner.
- Preserve exact legacy context IDs, private keys, serialized rows, export and restore.

**Non-Goals:**

- Changing runtime compaction policy, request assembly, transcript projection or provider execution.
- Adding an independent private-state saga, a generic mutation framework or a new usage-ledger schema.
- Rewriting historical v5 rows or inferring operations, producers or sessions they never recorded.

## Decisions

### Use a forward v6 schema with exclusive provenance columns

`context_summaries.turn_id` becomes nullable and gains nullable `operation_id`
and `producer_actor_id`. Typed context and reasoning records accept an optional
turn and optional operation and require exactly one. Operation context records
also require the producer actor. Existing rows remain unchanged. Editing the
already-used v5 migration or storing an operation in `turn_id` was rejected
because either choice falsifies historical/provenance meaning.

### Branch identity and serialization by provenance kind

Turn-attributed records execute the exact existing ID, state-key and serialized
record-format functions. Operation-attributed records use a new domain tag and
include the real operation and producer binding where applicable. Optional new
fields use compatible serde omission/defaults, so legacy JSON bytes and keys do
not change. A single Option-shaped hash function was rejected because it would
silently recalculate deployed identities.

### Extend the existing checkpoint envelope and transaction

`checkpoint_context_summary` accepts one typed envelope containing the context
record and zero or more private reasoning records. The store validates and
pre-encodes every sidecar key/value before opening the SQL transaction. Under
the existing view guard and transaction it rechecks revision, source range and
cursor, checks every sidecar identity conflict, then inserts sidecars, context
record and cursor before one commit. The existing view-operation receipt covers
the serialized whole envelope. A separate `put_reasoning_summaries` call was
rejected because its own commit can stale or partially precede the checkpoint.

### Bind the complete envelope before backend selection

One shared validator checks context invariants, sidecar count/component bounds,
session/producer/invocation/operation equality and exact serialized envelope
bytes. The facade runs it before matching Local/Remote and the owner repeats it.
The accepted cap is 64 MiB for the complete DTO, leaving fixed deterministic
headroom below the 100 MiB RPC frame even under JSON escaping. Summing only raw
string lengths was rejected because it does not bound transport expansion.

## Integration contract

`packages/kuru-memory` owns the DTOs, validation, migration, local store method,
facade routing and private service RPC. The public facade method accepts the
same typed checkpoint envelope for local, remote, main and candidate views; the
RPC carries that envelope without exposing SQL or backend handles. Existing
logical receipt identity hashes the complete serialized view operation. Runtime
compaction supplies the real session, producer actor, invocation and operation
through this method after this prerequisite lands.

The v6 migration reconciles optional turn and operation IDs explicitly. A
legacy turn value remains a required string on existing rows and decodes through
the old record format; an operation value is never inferred from it. New
operation records require the producer actor and use the new record format.
Local, managed and export/restore fixtures use the same typed DTOs and exact
identity helpers, so no fixture-only schema or parallel RPC representation is
introduced.

## Risks / Trade-offs

- **[Risk] Compatibility branching accidentally changes legacy identity.** → Keep explicit legacy code paths and byte-for-byte ID/key/JSON migration and restore fixtures.
- **[Risk] A sidecar conflict publishes part of the checkpoint.** → Pre-encode first, conflict-check and insert all effects inside one transaction, and test a conflict on a later record.
- **[Risk] Facade and owner accept different limits.** → Share one validator and exercise local, remote and direct-owner boundaries with exact-limit and one-byte-over payloads.
- **[Risk] Schema versions become conflated.** → Name the main v6 and usage v4 constants separately and verify fresh, upgraded and reopened stores.
