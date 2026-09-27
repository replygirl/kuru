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
- 9.2 reconciliation (2026-09-26): `origin/feat/windows-arm64` at `f1442d45` sits directly on #111 (`58540e0a`) and
  leaves `packages/kuru-delivery/src/coverage.rs` and `coverage/` byte-identical to #111. Its `EXCLUDED_ARTIFACTS`
  row for `aarch64-pc-windows-msvc`/`kuru-delivery/cospec_contract` exists only as a design contingency in its cospec
  artifacts, not in code, so there is no table to reconcile yet. When PR6b rebases onto this change it adds that row
  (if still needed) to the empty `EXCLUDED_ARTIFACTS` here and a `("windows-11-arm", Uninstrumented, N)` row to
  `PARTITIONS`, and rewrites its `SHARDS` matrix assertion against `PARTITIONS`.
