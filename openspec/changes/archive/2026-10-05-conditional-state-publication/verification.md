# Verification

Authored before implementation. Native hosted and local results remain distinct.

## 1. Atomic comparison and rollback [critical]

- [x] 1.1 @integration (agent) real Local and managed Remote races, absent seed and fresh retry -> one winner, typed stale loser, monotonic versions and all winners retained
- [x] 1.2 @integration (agent) real mixed stale batches and invalid/duplicate requests -> values, receipt count and revision unchanged
- [x] 1.4 @integration (agent) real negative and i64::MAX stored versions, including unconditional companions -> no partial value, receipt or revision changes; equal-value overwrites invalidate versions
- [x] 1.3 @integration (agent) unconditional/checkpoint and candidate writes -> old expectations fail; candidate versions stay private until exact promotion

## 2. Exact recovery and definite refusal [critical]

- [x] 2.1 @integration (agent) accepted conditional write with dropped reply -> exact recovery succeeds without replay; a later mutation remains possible
- [x] 2.2 @integration (agent) typed managed stale fault and subsequent fresh write -> stale metadata preserved and no uncertainty fence remains

## 3. Schema and compatibility [critical]

- [x] 3.1 @integration (agent) cold upgrade from previous registry with state and legacy topology -> values retained at version zero, current schema validated
- [x] 3.2 @integration (agent) previous registry writable open of upgraded store -> refusal and complete durable snapshot unchanged

## 4. Bounds, wire and repository checks

- [x] 4.1 @integration (agent) real ordered/missing batch reads and historical versioned API refusal -> one batch preserves order/missing values; old schema unchanged
- [x] 4.4 @integration (agent) exact 16 MiB encoded conditional payload and cumulative stored-JSON read bounds -> invalid requests never enter mutation/receipt; oversized rows/batches refused before retention; scalar reads remain compatible
- [x] 4.2 @unit (agent) operation contract and golden protocol pin -> read/write/receipt/fault classifications explicit with changed minor
- [x] 4.3 @integration (agent) package-owned focused and owning memory tests, host/Windows-target static checks, formatting and docs -> relevant checks pass with actual results recorded

- [x] 4.5 @integration (agent) Local and managed coherent versioned batches -> values and versions share a snapshot, requested order/missing keys and binary spelling remain intact; historical value-only read works and historical versioned batch refuses unchanged

## Observed local evidence

All commands ran from the owning isolated worktree at base `d9cfcb68fcffd3f2c821edbfdb9655c42f774d82`. Real-memory commands used the existing `KURU_DOLT_CACHE` override with the owning `target/test-engine-cache` and a process-local file-descriptor limit of 4096. The original shared cache was retained: its old inventory failed prefetch; the isolated cache then exposed the shell's 256-descriptor limit before this test invocation setting was applied. No private-object validation or fixture inventory was weakened.

- The first complete `mise run //packages/kuru-memory:test` finished with 725 library tests passing, 14 failing and 6 existing measurement tests ignored. The 14 failures were obsolete schema/protocol/count/ancestry fixture assertions after the additive schema 9 and wire 1.8 changes. They were corrected without changing production behavior or historical writable refusal. The bundle (10), memory (5), server-lifecycle (12) and supervisor-snapshot (1) integration targets passed. Final affected-fixture reruns are recorded below; this is not a claim that a second full suite passed.
- `KURU_BLESS_PROTOCOL_PIN=1 mise run //packages/kuru-memory:test -- protocol_surface`: final versioned-batch surface regenerated through the documented gate; exit 0, 1 test passed, wire 1.8.
- `mise run //packages/kuru-memory:test -- conditional_state`: final source exit 0, 5 tests passed. These exercise real local/managed races, stale rollback and fence clearing, exact dropped-reply recovery, equal-value/checkpoint/candidate invalidation, corrupt/overflow checkpoint rollback, binary keys, coherent versioned batches, bounds and byte-preserving schema-8 upgrade/historical refusal.
- `mise run //packages/kuru-memory:lint`, `mise run //packages/kuru-memory:lint:windows`, and `mise run format:rust`: final source exit 0 for each.
- `mise run //packages/kuru-memory:test -- store::migrations`: 39 passed, 1 failed, 2 existing measurements ignored. The remaining exact-v8 fixture correctly upgraded with the released-v8 registry but still classified with the current-v9 helper, producing `historical Dolt migration branch unexpectedly names active main`. Its classifier now uses the same exact-v8 registry and target; the original 7-record/1-dirty-attempt assertions remain intact. The final single-fixture rerun is recorded below.
- `mise run //packages/kuru-memory:test -- v8_backfills_every_published_branch_from_classification`: exit 0, 1 passed after the reviewer-approved fixture-only classifier correction. Together with the 39 passing migration tests, all affected migration checks have passed; the two explicitly ignored measurements were not run.
- `mise run //packages/kuru-memory:test -- released_v1`: exit 0, 3 passed, including both corrected ordered-ancestry probes and dirty/future-marker refusal.
- `mise run //packages/kuru-memory:test -- starter_token_is_an_optional`: exit 0, 1 passed with wire 1.8 in the exact hello fixture.
- `mise run //packages/kuru-memory:test -- template_key_tracks`: exit 0, 1 passed with the schema-9 definition inventory.
- `mise run //packages/kuru-memory:test -- service::rpc::contract_tests`: exit 0, all 6 passed, including exhaustive classifications and the final exact wire 1.8 pin.
- `mise run //apps/kuru-docs:check`: exit 0 for build, formatting, lint and published-site link/anchor checks. No documentation was deployed.
- `mise run cospec:managed:check`: exit 0, no managed drift. Required strict validation and actual apply gates passed before implementation and again before the added coherent versioned-batch seam.

Native hosted acceptance, workspace coverage and the complete final suite remain CI checks; local Windows-target lint is not native Windows runtime evidence.

Independent source/design review accepted the final versioned-batch delta and the final exact-v8 fixture-only classifier correction. The scoped reruns above prove every corrected failure from the first full run. No broader runtime/topology implementation or product acceptance is claimed by this foundation change.
