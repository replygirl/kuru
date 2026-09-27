# Dependencies

## Blocked by

- [x] `coverage-shard-orchestrator` — the Rust `coverage shard`/`coverage collect` orchestrator, the injectable `Host` boundary, Unix group supervision, OS-labelled receipts and the per-OS shard/collect/install workflow this change partitions (PR #111; this branch is stacked on its head and moves onto `main` once #111 merges) *(archived 2026-09-26)*

## Soft-blocked by

None.

## Coordination

- PR6b `feat/windows-arm64` (`windows-arm64-support`, not an active change in this tree) reserves a host-keyed
  `EXCLUDED_ARTIFACTS` table in `packages/kuru-delivery/src/coverage.rs`, wants uninstrumented `windows-11-arm`
  legs, and pins its matrix to `SHARDS`. This change keeps `EXCLUDED_ARTIFACTS` an independent constant (consumer
  implemented with an empty table), provides the uninstrumented partition mode, and replaces `SHARDS` with
  `PARTITIONS`; whichever lands second rewrites the other's matrix assertion. Before pushing, diff that branch's
  `coverage.rs` against #111 and reconcile the tables.
