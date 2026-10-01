# Proposal

## Why

The cold staged build of a new memory store (a legacy import, a configured
`memory.dolt_binary`, a busy or unreadable template key lock, a damaged
template, or a `Creation::Cold` test fixture) runs three separate Dolt engine
starts in its private staging directory before the active start: `init`
(bootstrap, `initialize`, the optional legacy import), `migrate` (the schema
chain), and `validate_and_mark` (`validate_active`, the revision read and
`ready.json`). Each start pays its own process spawn, readiness wait and main
pool authentication, and each close pays its own quiescence. The template
build job (`StageWorker::build_template`) already proves that bootstrap,
`initialize`, the migration chain and validation run correctly on one engine;
the cold project path pays the same chain spread across three engines and two
extra closes for no additional safety, since the worker already owns the
startup lock as its reap guard across job boundaries and a cancelled opener
already cannot abandon DDL or release the lock before the reap.

## What Changes

- Collapse the cold staging path's three engine starts (`StageWorker::init`,
  `StageWorker::migrate`, `StageWorker::validate_and_mark`) into one staging
  job, in the shape `StageWorker::build_template` already uses: bootstrap and
  `initialize` (and the optional legacy import, which `build_template` never
  carries) on the fresh stage, `migrations::upgrade` (the chain),
  `validate_active`, the revision read, and `ready.json` written between the
  marker boundaries, then one close and reap. The worker still owns the
  server from hand-over, so the startup lock is still the server's reap
  guard and still returns only after that reap.
- `MemoryStore::create_cold` (`store.rs`) calls the new single job once
  instead of `init` → `migrate` → `validate_and_mark` in sequence, then keeps
  quiescence, rename and the unchanged active start exactly as today; the
  active start still runs `validate_active` after the reload from disk.
- Failure and cancellation semantics are unchanged in effect: a failure
  before `ready.json` still leaves an unready stage that the next open's
  recovery preserves without a start; a failure after `ready.json` still
  leaves a ready stage the next open reuses through the inspection path; a
  cancelled opener still cannot abandon DDL or release the lock before the
  reap. Existing error messages stay where the step they name still exists.
- `FreshOpen::Cold` (`test_support.rs`) drops from 4 engine starts (3 closes)
  to 2 (1 close); `fresh_open_budget`, `fixture_deadline` and every budget
  derived from them shrink with it, as the "Existing tests that must change"
  table of the design doc requires.
- Retarget `store/open_pool_budget_tests.rs` (the delayed-pool hook moves
  from the old validate start to the single cold job's pool),
  `store/recovery_tests.rs` and `store/migration_lifecycle_tests.rs`
  (migration-hook pause points that assumed a separate `migrate` start) to
  the new one-engine sequence, without weakening what they assert.
- Update `docs/memory.md`, `apps/kuru-docs/concepts/memory.md` and
  `docs/development.md` wherever they describe the cold creation path as
  "four database starts" or "four starts"; they become two.
- No change to `openspec/specs/versioned-memory/spec.md` requirement text:
  "Store creation path selection" already states only that "the cold staged
  build keeps its own engine starts" without naming a count, and "Current-
  schema staging and preserved failures" already describes a single "owned
  live staging session" that validates, publishes and then stops and reaps.
  Both stay true as written; only the internal engine-start count changes,
  which is why this ships as `perf`, not `feat`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None — no requirement text in `openspec/specs/` changes; see Why/What
Changes.

## Impact

- `packages/kuru-memory/src/store/stage_worker.rs`: replace `init`,
  `migrate` and `validate_and_mark` with one cold staging job (new or
  renamed method) built on the `build_template` shape; update its module
  doc comment's job list.
- `packages/kuru-memory/src/store.rs`: `create_cold` calls the single job;
  no change to `open_inner`'s template-path branch, quiescence, rename or
  active-start logic.
- `packages/kuru-memory/src/test_support.rs`: `FreshOpen::Cold::starts()`
  (4 → 2) and its doc comment; `fresh_open_budget_of`, `fresh_open_budget`,
  `fixture_deadline` follow from the constant.
- `packages/kuru-memory/src/store/open_pool_budget_tests.rs`,
  `packages/kuru-memory/src/store/recovery_tests.rs`,
  `packages/kuru-memory/src/store/migration_lifecycle_tests.rs`: hook and
  pause points retargeted to the new single-engine cold job.
- `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`,
  `docs/development.md`: cold-path start counts corrected from four to two.
- No change to `packages/kuru-memory/src/store/migrations.rs` (the
  migration registry, owned by parallel work on another branch), no change
  to the template build or template copy paths, no change to timeouts,
  deadlines, retries or recovery classification of existing stage states.

## Benchmarks

| Metric | Before | After | How measured |
|---|---|---|---|
| Cold staged build: engine starts | 4 | 2 | `test_support::engine_ledger`, counted by `cold_stage_initializes_migrates_and_validates_on_one_engine` |
| Cold staged build: owned closes before ready | 3 | 1 | same ledger, same test |
| Cold staged build: in-process wall time (legacy import fixture) | [to measure: baseline run on this branch before the change] | [to measure: run on this branch after the change] | in-process benchmark harness, `cargo bench`-style timing inside `packages/kuru-memory`, same machine, same warm engine cache, median of N≥5 runs (same methodology as the design doc's section 2.5 "derived, fragile" figures) |

The wall-time row is recorded as `[to measure]` here; task 4 in `tasks.md`
runs the benchmark and fills in the observed before/after numbers and N.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
