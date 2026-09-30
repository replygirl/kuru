# Proposal

## Why

Every open of an existing project, and every migration step, re-inspects each
retained migration branch by opening two SQL connection pools per branch: one
on the branch itself (`Server::pool(&name)`) and one on its parent commit
(`Server::pool(commit)` inside `validate_commit_version`)
(`packages/kuru-memory/src/store/migrations.rs:1502-1573, 1601-1610`). This
runs on every writable open (`validate_active`), every read-only open
(`validate_inspection`), every recovered ready stage (`validate_ready`), and
every per-step migration loop (`upgrade_in`) — for every retained branch, on
every open, not only when a store is first created. Each pool pays a fresh
Dolt connection, `USE` and the per-connection project/instance identity check
(`server.rs:1564-1614`), which also means a branch whose instance identity
differs from main's (a template-born store, the coming work in a sibling
change) cannot be classified at all today: the per-connection identity check
refuses the pool before any query runs.

## What Changes

Replace the per-branch and per-parent pool opens in
`classify_historical_attempts_in` and `validate_commit_version` with queries
issued from the existing main connection pool the caller already holds:
`AS OF '<hash>'` reads for schema version and receipts, `dolt_branches` for
head hash and the `dirty` flag, a revision-qualified
`` `kuru/<branch>`.dolt_status `` read for the working-set shape check,
`dolt_commit_ancestors` for the sole-parent check, and `dolt_log` for
ancestry (already main-pool queries today). The one check with no drop-in
main-pool equivalent is full schema validation (`validate_version_with`) at
the branch head and at the parent: `information_schema` scoped from main
returns no columns for a non-`DATABASE()` schema on the pinned engine. The
first implementation task settles which of three forms
(`USE kuru/<hash>` on a detached main-pool connection with the existing
validator queries; `SHOW CREATE TABLE ... AS OF`; or `information_schema`
qualified by revision or a revision-qualified database name) reproduces
`validate_version_with`'s verdict exactly, including for a parent with a
wrong schema, before the rest of the classifier is written against it. Any
main-pool query that returns no row or an unexpected shape fails closed
(ambiguous), never as published — no check is weakened to make a main-pool
form fit. The new classifier replaces the call sites in `validate_active`,
`validate_inspection`, `validate_ready` and `upgrade_in`'s per-step loop; no
change to what is validated, to migration steps, to publish, staging,
provisioning, service, timeouts, deadlines or retries.

## Capabilities

### New Capabilities

(none — no new capability; classification's observable behavior and spec
text are unchanged)

### Modified Capabilities

(none — `versioned-memory` requirements already state branches are
classified "from their committed receipt, registered step and ancestry"
(`openspec/specs/versioned-memory/spec.md:188-189`); this change makes that
true with fewer connections, it does not change what is required or
observable. A delta is added only if the parity task below cannot reproduce
today's verdict with a main-pool-only design, which would surface as an
implementation-blocking finding, not a requirements change.)

## Impact

- `packages/kuru-memory/src/store/migrations.rs`: `classify_historical_attempts_in`,
  `validate_commit_version`, `validate_version_with`, `validate_schema_with`,
  `validate_receipts`, `ancestor`, `sole_parent`, `inventory_with`/`inventory_in`
  (read as reference only), and the four call sites `validate_active`,
  `validate_inspection`, `validate_ready`, `upgrade_in`.
- Existing tests that assert pool/query counts change to the new counts;
  tests asserting classification verdicts stay green unchanged.
- No change to `packages/kuru-memory` public `MemoryStore`/`Server` APIs, to
  migration step definitions, to `docs/memory.md` /
  `apps/kuru-docs/concepts/memory.md` observable behavior (docs are updated
  only if their described mechanism, not just its cost, changes), or to
  `openspec/specs/versioned-memory/spec.md` normative text.

## Benchmarks

| Metric | Before (measured) | After (to record) | How measured |
|---|---|---|---|
| `Server::pool` opens per classified retained branch, on `validate_active` / `validate_inspection` (reader) / `validate_ready` | 2 per branch (1 branch pool + 1 parent-commit pool), clean path; 1 per branch, dirty path | 0 | `test_support::engine_ledger` counting hook on `Server::pool`, asserted in the new `historical_classification_opens_no_branch_or_commit_pools` test (task 2) |
| Wall-clock cost of classifying N retained branches on an existing-project open | not separately measured today (folded into open time) | recorded, not gated — no product deadline changes | local timing note beside the pool-count assertion, for the findings file only; not a pass/fail criterion (this change does not touch `startup_timeout_secs` or any deadline) |

The primary, gating metric is the pool count, because it is deterministic and
directly reflects the removed work (a Dolt connection, `USE`, and an
identity check per pool); wall-clock time on a developer machine is recorded
for context only, per SD's existing position that report-only timing is not
itself a pass criterion (SD §9, test S10).

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
