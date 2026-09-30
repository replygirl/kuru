# Proposal

## Why

`MemoryStore::open_inner`'s fresh-store branch (`packages/kuru-memory/src/store.rs`)
interleaves three staging engine starts (init, migrate, validate-and-mark) with
the startup-lock and lease bookkeeping that makes each start safe to cancel.
The coming per-machine template work (P4b) needs to run the same init/migrate/
validate sequence on a template build engine, not just a project's staging
engine, and it needs `OpenOptions` to carry a creation-path selector so tests
can force the cold path once a warm-template path exists, without giving any
product caller the ability to set it. Extracting the staging pipeline into an
explicit, job-shaped worker now — before the template PR needs to reuse it —
keeps that later change additive instead of requiring a second pass through
`open_inner`'s cancellation-sensitive control flow.

## What Changes

- Add `packages/kuru-memory/src/store/stage_worker.rs`, owning the three
  staging-pipeline steps of today's fresh-store `open_inner` branch as
  explicit jobs on one worker: `init` (staging directory creation, supervisor
  start, bootstrap, `initialize`, optional legacy `import`), `migrate` (reopen
  and run today's `run_migration_worker`, which owns and reaps its own
  server), and `validate-and-mark` (reopen, `validate_active`, `revision`,
  the `ready.json` write between the existing `marker_fixture` Before/After
  boundaries). Each job keeps today's exact start/close pattern: its own
  `Server::open_with_guard`, its own `close_migration_worker` or
  `close_failed_open`, and today's `preserve_unready_stage` call on failure.
  `open_inner` calls the worker for the fresh-store path only; moving the
  stage to the active path (quiescence, `move_to`), the active-path start and
  everything after it are unchanged and stay in `open_inner`.
- Add a `Creation` enum (`Default`, `Cold`) as a new field on `OpenOptions`,
  crate-private and gated `cfg(any(test, feature = "test-support"))`, so no
  product caller can construct a non-default value (`OpenOptions` is never
  built by struct literal outside `packages/kuru-memory`). Today both
  variants take the same code path — `open_inner` does not yet branch on it —
  the field only exists so a later change can route `Creation::Default`
  through a template. `test_support::temporary_cold()` sets
  `Creation::Cold`; `temporary()` and `open_temporary()` keep `Creation::Default`.
- Add `cancelled_open_during_stage_build_keeps_startup_lock_until_reap`: cancel
  an opener mid stage-build and show a second opener acquires the startup
  lock only after the engine ledger records the reap.

## Capabilities

### New Capabilities

None — this reshapes an internal implementation module; it adds no new
observable capability.

### Modified Capabilities

None. Every engine start, close, lock, lease, marker boundary, error message
and cancellation behaviour of a fresh store open stays exactly as it is
today: 4 engine starts, 3 closes and 10 process launches for a fresh open.
No `openspec/specs/**` requirement changes; this is a structural move with
identical externally observable behaviour.

## Impact

- `packages/kuru-memory/src/store.rs`: `open_inner`'s fresh-store branch
  shrinks to call the new stage worker and keep the move-to-active and
  active-open steps; `OpenOptions` gains the crate-private `Creation` field.
- `packages/kuru-memory/src/store/stage_worker.rs` (new): the extracted
  `init`, `migrate`, `validate-and-mark` jobs.
- `packages/kuru-memory/src/test_support.rs` and `store.rs`'s
  `temporary()` / `temporary_cold()` / `open_temporary()` (13 `temporary_cold()`
  call sites measured at 0b39e733; the research's 15 counted mentions): thread `Creation` through.
- No change to `packages/kuru-memory/src/server.rs`, `store/migrations.rs`,
  `provision.rs`, or any public API outside `kuru-memory`.
- `docs/development.md` only if it names the moved functions by path; no
  user-facing documentation change is expected.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
