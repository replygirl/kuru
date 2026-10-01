# Proposal

## Why

A writable open still decodes every ledger-owned usage row, twice, before provider dispatch. Index-range paging (archived `usage-scan-index-range`) made that walk linear (about 7.8 microseconds per row on the Ubuntu CI job, measured on run 36915490039), but every open still pays O(rows) for the life of the project, and the first open after each release would pay it with no way to avoid it.

The scan guards bytes the ledger's own writer cannot produce (an older or newer binary, a manual edit, corruption). The guarantee that matters is that the pool is published only when every owned row of the exact state being activated is known to pass the current validator. That can be proved by a durable record bound to the content and to the validator, written in the same Dolt commit that produced the content, so a recorded reopen needs no row decode at all.

## What Changes

- Factor the owned-row check into one function, `validate_owned_row`, used by the open scan and by every ledger write before its `INSERT`.
- Every ledger write records a `Kuru-Usage-State: <validator id> <DOLT_HASHOF_TABLE('state')>` trailer in the same `DOLT_COMMIT` that produced the content it names.
- A writable open reads the trailer on HEAD under the write guard. A trailer naming this binary's validator and the live `state` hash is Bound and decodes no rows. Anything else is Missing: run the full scan, then record the result with one empty commit, only when at least one owned row exists. A missing, foreign or mismatched record never refuses an open by itself.
- Every ledger write carries a precondition: the live `state` hash must equal the hash this open validated. On inequality it rolls back, returns a typed error, and later writes refuse until a reopen revalidates.
- The record commit and the writes' commits register uncertain outcomes with their own receipts (`Receipt::UsageValidation`, `Receipt::UsageOperation`). After `resolve_uncertain` settles a usage write, the in-memory hash is re-derived from HEAD's trailer.
- D: after `upgrade_usage` and `validate_usage`, the second owned walk is replaced by a re-read of the `state` hash; the walk runs again only if the hash changed.
- The validator id is `kuru.usage.state.v1+<release version>`, so the first open after an upgrade runs one full linear scan and re-records. A golden corpus pins the validator's verdicts for development builds.
- The `usage-scan-scaling` CI job gets calibrated bounds with their derivation, forces the full-scan path in its driver (an empty foreign commit before each full sample), splits full and bound sample series, and asserts that a recorded reopen decodes 0 rows.
- `docs/memory.md` documents the trailer as a durable convention and the one-time re-check after an upgrade.
- No DDL, no schema version change, no change to read-only opens. Older binaries ignore the trailer (see the design).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `context-usage-accounting`: the "Invalid operational branch" scenario names ledger-owned rows among what must validate.
- `memory-store-lifecycle`: "Permanent operational usage branch lifecycle" gains the content-bound validation rules; a new requirement states the validation record as a durable convention with its trailer format and the scan-and-re-record rule.

## Impact

- `packages/kuru-memory/src/store/usage_ledger.rs`: `validate_owned_row`, trailer encoder and parser, `bound_validation`, `establish` order, the write precondition and trailer, module-doc invariants, golden corpus.
- `packages/kuru-memory/src/store.rs`: `Shared.usage_validated`, `Receipt::UsageValidation` and `Receipt::UsageOperation` with their `resolve_uncertain` arms and the post-settle re-derivation.
- `packages/kuru-memory/src/open_timeline.rs`: a count for the full-path rows of a D-forced rescan if needed (additive, format 1 unchanged).
- `packages/kuru-memory/src/test_support/usage_scan.rs` and its tests: forcing step, full and bound series, calibrated bounds, the enabled 0-rows assertion.
- `.github/workflows/ci.yml` (`usage-scan-scaling` job text), `docs/development.md`, `docs/memory.md`.
- Behavior changes: usage commit messages gain a trailer; non-empty unrecorded ledgers gain one validation commit once; a write that finds `state` changed under the writer now refuses (today it proceeds). Not breaking for older binaries.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
