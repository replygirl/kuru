# Proposal

## Why

The cold staged build of a new memory store (a legacy import, a configured
`memory.dolt_binary`, a busy or unreadable template key lock, a damaged
template, or a `Creation::Cold` test fixture) ran three separate Dolt engine
starts in its private staging directory before the active start: `init`
(bootstrap, `initialize`, the optional legacy import), `migrate` (the schema
chain), and `validate_and_mark` (`validate_active`, the revision read and
`ready.json`). Each start paid its own process spawn, readiness wait and main
pool authentication, and each close its own drain and reap. The template
build job (`StageWorker::build_template`) already proves that bootstrap,
`initialize`, the migration chain and validation run correctly on one engine;
the cold project path paid the same chain spread across three engines and two
extra closes for no additional safety: the startup lock is each engine's reap
guard either way, and the stage is validated again by the active start after
the reload from disk.

## What Changes

- The cold staging path's three engine starts (`StageWorker::init`,
  `StageWorker::migrate`, `StageWorker::validate_and_mark`) become one job,
  `StageWorker::build_cold`: create the stage, start its engine (bootstrap),
  then hand the server to a staging worker task that runs `initialize`, the
  optional legacy import at schema 1, `migrations::upgrade` (the chain),
  `validate_active`, the revision read and `ready.json` between the marker
  boundaries, then closes and reaps. The worker owns the server from the
  hand-over (the `run_migration_worker` shape), so the startup lock is still
  the server's reap guard and returns only after that reap.
- `build_template` and the cold job share one `initialize`, import and chain
  step (`build_schema`) instead of a third copy of the sequence; the adopted
  template stage (`adopt_and_mark`) and the cold job share the validation,
  marker, close and preservation session (`StageSession`).
- `MemoryStore::create_cold` (`store.rs`) calls `build_cold` once, then keeps
  quiescence, rename and the unchanged active start; the active start still
  runs `validate_active` after the reload from disk. `open_inner` holds the
  prepared legacy import in an `Arc` so the worker task can own it.
- Failure semantics are unchanged in effect: a failure before `ready.json`
  preserves the unready stage (now always by the staging worker, while it
  still holds the lock); a failure after `ready.json` leaves a ready stage
  the next open reuses through the inspection path. Each step's error texts
  are kept ("open staged memory server", "open staged main pool", and the
  initialization, migration and activation preservation and cleanup texts);
  the reopen texts of the two removed starts are gone with them.
- Cancellation: a cancelled opener still cannot abandon DDL or release the
  lock before the reap. Because the worker now owns the whole session, an
  opener cancelled during initialization, migration or validation leaves the
  worker to finish, as the template path's creation worker already does: the
  stage it leaves is ready (and reused) or preserved. Before this change a
  cancellation outside the migration step dropped the server and left an
  unready stage for the next open to preserve.
- Test budgets: `FreshOpen::Cold` drops from 4 starts (3 closes) to 2 (1).
  `fixture_deadline` budgets two starts per fresh open (the reviewed values
  become 126, 158, 222 and 318 s), since a fixture open never builds the
  template. `fresh_open_budget` becomes the first project's (3 starts, now
  the longest path). `template_warm_up_bound` keeps its value (four starts
  and three closes), written out explicitly. These budgets are now
  `#[cfg(test)]`.
- Tests: the stage pool delay hook (renamed `stage_pool_delay`) moves to the
  one cold start's pool; `open_error_reap_tests` asserts the cold start's
  "open staged main pool". The accepted-DDL cancellation case now expects
  the worker to finish and the next open to reuse the ready stage. Recovery
  and migration-lifecycle tests needed no change: their migration hooks
  pause the same chain inside the one session. New tests below.
- Docs: `docs/memory.md`, `apps/kuru-docs/concepts/memory.md` and
  `docs/development.md` no longer say the cold path takes four starts.
- No change to `openspec/specs/versioned-memory/spec.md` requirement text:
  "Store creation path selection" states only that "the cold staged build
  keeps its own engine starts", and "Current-schema staging and preserved
  failures" describes one "owned live staging session" that validates,
  publishes the activation record and then stops and reaps. Both stay true as
  written, which is why this ships as `perf`, not `feat`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None — no requirement text in `openspec/specs/` changes; see What Changes.

## Impact

- `packages/kuru-memory/src/store/stage_worker.rs`: `build_cold`,
  `build_schema`, `StageSession`, the test-only `StageHooks` and
  `ValidationProbe`; module doc; new tests.
- `packages/kuru-memory/src/store.rs`: `create_cold` calls `build_cold`;
  `open_inner` wraps the legacy import in an `Arc` and records the active
  validation for the test probe; `OpenOptions` test fields
  `stage_pool_delay` and `validation_probe`. The template-path branch,
  quiescence, rename and active-start logic are unchanged.
- `packages/kuru-memory/src/store/creation_worker.rs`,
  `creation_template.rs`, `template_stage_tests.rs`: `StageWorker`
  construction (`hooks: StageHooks::default()`); creation worker module doc.
- `packages/kuru-memory/src/test_support.rs`: `FreshOpen::Cold` and the
  budgets above.
- `packages/kuru-memory/src/store/open_pool_budget_tests.rs`,
  `open_error_reap_tests.rs`, `creation_template/open_tests.rs` (doc only):
  retargeted to the one cold start.
- `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`,
  `docs/development.md`.
- No change to `packages/kuru-memory/src/store/migrations.rs` (the
  migration registry, owned by parallel work on another branch), to the
  template build or template copy paths, to production timeouts, deadlines or
  retries, or to the recovery classification of existing stage states.

## Benchmarks

| Metric | Before | After | How measured |
|---|---|---|---|
| Cold staged build: engine starts | 4 | 2 | `test_support::engine_ledger` (`Ledger::starts_under`): `cold_stage_initializes_migrates_and_validates_on_one_engine` (red on the base with 3 starts at the ready marker, green after); in-process timing rows below, 10 of 10 opens each side |
| Cold staged build: owned closes before ready | 3 | 1 | same test: at the boundary after `ready.json`, 1 start and that engine still live |
| In-process `Creation::Cold` open, interleaved base/head (2 rounds of 5 each) | p50 3099 ms (min 2675, p90 3343, N=10) | p50 2409 ms (min 2313, p90 2659, N=10) | an ad hoc timing harness around `spawn_gated_open` with a warmed cache, debug test executables of base `46c2d60c` and head `ef86dc74` run alternately on one host; 1-min load 6.1–6.8 on 14 CPUs. That interleaved base/head comparison itself is not committed (it needs two checkouts' executables); the single-checkout timing shape it used is now committed and reproducible as `stage_worker::tests::measure_cold_open_latency` (`mise run //packages/kuru-memory:measure:cold-open`), which times this same `spawn_gated_open` sequence for a `Creation::Cold` open and a legacy import, head-only |
| In-process legacy-import open, same interleaved run | p50 3129 ms (min 2560, p90 3245, N=10) | p50 2409 ms (min 2203, p90 2531, N=10) | same ad hoc harness, a one-message legacy SQLite source; reproducible head-only through the same committed `measure:cold-open` task |
| Same two opens, sequential runs (base, then head after the build) | cold p50 2549, legacy p50 2568 (N=10 each) | cold p50 2629, legacy p50 2498 (N=10 each) | same ad hoc harness, not interleaved; 1-min load 12.4→7.6 (base) and 7.6→6.8 (head); head spread sd 158 and 275 ms against base 48 and 56: within noise, not evidence either way |
| Release open-time harness (first-launch / new-project / cold-existing / warm-reopen) | starts 3 / 2 / 1 / 1; p50 6407 / 1242 / 489 / 496 ms | starts 3 / 2 / 1 / 1; p50 7096 / 1377 / 574 / 537 ms | `kuru-delivery open-time` (`ci/open-time-report` `456e4ba1`), N=10 per case per side; none of these cases takes the cold path, so the counts are unchanged as expected; the head series ran under higher load (per-run 1-min 8.4–13.4 against 5.1–12.1, largest sampling bracket 1131 ms against 120 ms), so its times are not comparable |

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
