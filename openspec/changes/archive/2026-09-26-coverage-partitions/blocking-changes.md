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

## Follow-on changes

Tracked here because this change is archived; each needs its own cospec change before implementation.

- [ ] `coverage-instantiation-gate` — design D4a option (iii). Each partition exports per-instantiation mapped and
  covered line sets from `llvm-cov export`, self-checked per file against llvm-cov's own `LF`/`LH` and failing
  closed on any mismatch; the merge unions covered sets per instantiation and takes each group's maximum, so the
  merged per-OS gate reproduces cargo-llvm-cov's `--fail-under-lines 90` exactly and `LINE_GATE_PERCENT` returns to
  90 of that metric. Until then the merged gate is option (ii), 91% of unique instrumented lines, an interim margin
  over a measured (0.67-0.75 point), not bounded, difference; AGENTS.md and `docs/development.md` state this. The
  lead's confirmation of (ii) as the interim is requested on PR #118 and is not recorded here.
- [ ] `windows-verify-native-build-partitions` — design D9, the Windows `bundle:verify-native-build` half of scope
  item (4) (task 5.2b). Open lead decision among (a) leave it in the Windows install job, (b) its own parallel
  Windows job, (c) a `-p kuru-memory` valid-archive build agreed with kuru-memory's owner.
