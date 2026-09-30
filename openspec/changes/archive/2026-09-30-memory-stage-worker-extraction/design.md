# Design

## Context

`MemoryStore::open_inner` (`packages/kuru-memory/src/store.rs`) is one long
async function. Its fresh-store branch (no project yet at `directory`) runs,
in order, on a `<name>.staging-<uuid>` directory:

1. **init**: create the staging directory, `Server::open_with_guard` (start
   1), open the main pool, `initialize(&pool)` plus optional legacy
   `import(&pool, legacy)`, `close_migration_worker`. On failure,
   `preserve_unready_stage` moves the stage to `interrupted/`.
2. **migrate**: reopen the staging directory (start 2),
   `run_migration_worker(server, pool[, hooks])` — itself a `tokio::spawn`ed
   task that owns the server across `migrations::upgrade`, then calls
   `close_migration_worker` before replying over a oneshot channel, so a
   cancelled `open_inner` future cannot abandon DDL or the writer lock before
   the supervisor reaps. On failure, `preserve_unready_stage` again.
3. **validate-and-mark**: reopen the staging directory (start 3),
   `migrations::validate_active`, `revision(&pool)`, build the `Activation`,
   `marker_fixture::reach(..., Boundary::Before)`, write `ready.json`,
   `marker_fixture::reach(..., Boundary::After)`, `close_migration_worker`. On
   failure, `preserve_unready_stage` runs only if `ready.json` does not yet
   exist — once it exists the stage is a valid ready stage for a later
   `recover_staging` to pick up, not an interrupted one.

`open_inner` then quiesces the stage's lifecycle lease, renames it onto the
active path (`staging._lease.move_to(&directory)`), and opens the active
directory as start 4. The startup lock passed into `open_inner` is threaded
through every step as a `Server`'s reap guard (`lock.take()` /
`lock = Some(returned_lock)`); the per-machine template design
(`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`, section 7)
needs steps 1-3 available as a reusable worker so a template build can run
them once against a build-scoped directory instead of a project's staging
directory, on the same engine, without a second implementation of this
cancellation discipline.

## Goals / Non-Goals

**Goals:**

- Move the three staging-pipeline steps into `store/stage_worker.rs` as
  explicit jobs (`init`, `migrate`, `validate-and-mark`) on one worker type,
  each preserving its exact current start/close/preserve pattern and error
  text.
- Keep `open_inner` as the only caller for the project cold path; it drives
  the worker's jobs in sequence, then performs the move-to-active rename and
  the active-path open itself, unchanged.
- Shape the worker so a later job type (the template build: bootstrap,
  initialize, upgrade, validate, on one engine, no reopen between steps) can
  be added without re-touching the extracted `init`/`migrate`/
  `validate-and-mark` jobs — but do not add that job type in this change.
- Add a crate-private `Creation` selector (`Default`, `Cold`) on
  `OpenOptions`, gated `cfg(any(test, feature = "test-support"))`, with no
  behavioural branch on it yet.
- Add the cancellation test named in the proposal.

**Non-Goals:**

- No change to engine start count (4), close count (3), process launch count
  (10), lock/lease acquisition order, error text, progress-stage reporting,
  provisioning, migration step behaviour, recovery classification
  (`recover_staging`), or the marker-fixture boundaries.
- No restart removal (a separate, later change collapses steps 2 and 3 into
  one start — out of scope here; both stay as two starts).
- No template build job, no per-machine cache, no warm-up path (the
  per-machine template design's P4b).
- No records work (versioned-memory `records` capability, out of scope).
- No routing of `Creation::Default` through anything other than today's cold
  path; `open_inner` does not read the field yet.

## Decisions

- **Worker shape: one struct with three async methods, not three free
  functions.** `stage_worker::StageWorker` holds the values every job needs
  (the `ServerOptions` closure inputs, the staging `PathBuf`, `lifecycle_root`,
  `timeout`, `parent`, the legacy import and `project_scope`) so a future
  `TemplateBuild` job can be added as a fourth method without re-deriving
  those inputs at each call site. Rejected: three free functions taking the
  same seven-plus parameters each — it reproduces `open_inner`'s current
  parameter sprawl inside the new module instead of removing it, and gives
  the template PR nowhere to hang a build-scoped variant of the shared
  inputs.
- **Each job takes the startup lock `File` in and returns it out, exactly as
  `open_inner` threads it today.** A job takes `File` (the startup lock, or
  for a future build job, an exclusive key lock) and returns `Result<File>`;
  `open_inner` passes `lock.take().expect("startup lock")` to `init`, chains
  each returned `File` into the next job, and stores the last one with
  `lock = Some(...)` before quiescence, unchanged from today's
  `lock.take()` / `lock = Some(returned_lock)` pattern. On every error path
  today `open_inner`'s `lock` is already `None` (the `File` lives inside a
  `Server` or is dropped inside a failure arm), so passing the `File` by
  value changes no lock lifetime. Rejected: the worker owning the lock across
  all three jobs internally — that would change the visible lock lifetime
  between `preserve_unready_stage` calls (the lock is dropped inside
  `preserve_unready_stage`'s failure arms today) and risks silently altering
  when a cancelled opener's lock becomes observable to a second opener, which
  the new cancellation test exists to catch. Also rejected: `Option<File>` in
  and out — every caller has a lock, so the `Option` only adds an `expect`.
- **One private `start` helper opens each job's engine and main pool.** It
  reports `OpeningDatabase`, calls `Server::open_with_guard` with the lock as
  the reap guard, and opens the main pool with `close_failed_open` on
  failure. A private `Start` enum (`Init`, `Migrate`, `Validate`) keeps each
  start's exact `.context(...)` text and applies the test-only
  `migrated_stage_pool_delay` only to the validate start, between the server
  open and the pool open, as today. This is the seam the template build job
  reuses: it starts one engine and runs its steps on it.
- **`migrate` keeps calling today's `run_migration_worker` verbatim, not
  inlined into the new module's job body.** `run_migration_worker` already
  encodes the required "worker owns and reaps its server, independent of the
  awaiting future's cancellation" discipline (store.rs, `tokio::spawn` plus a
  oneshot reply). Moving its call site into `stage_worker.rs` is a plain
  relocation; `store.rs` keeps the function itself, since
  `run_candidate_recovery_worker` and the existing-store behind-schema path
  also call it and are out of scope for this change.
- **`Creation` lives on `OpenOptions` as a field, not a separate parameter
  threaded through `open_inner`.** `OpenOptions` already carries four
  `cfg(test)` fields set directly by test code in the same module
  (`migration_hooks`, `candidate_recovery_pause`, `candidate_cleanup_failure`,
  `migrated_stage_pool_delay`); `Creation` follows that existing pattern
  exactly, widened to `cfg(any(test, feature = "test-support"))` since
  `test_support` fixtures (built with that feature, not `cfg(test)`) are the
  actual callers. `open_temporary` gains a `creation` argument:
  `temporary_cold()` passes `Creation::Cold`, while `temporary()` and the
  test template's `build()` (`test_support/template.rs`) pass
  `Creation::Default`, since the template build is the open that later takes
  the copy path. Both variants are constructed explicitly (no derived
  `Default`). Rejected: a builder method (`OpenOptions::with_creation`)
  — none of the existing test-only fields use one, and adding one here alone
  would be an inconsistent precedent.
- **Verify no product construction site by grep, not by type-level
  enforcement.** `OpenOptions` has no `Default` impl and is never
  `#[derive(Default)]`'d, so the only way to build one is `OpenOptions::new`
  (which does not set `Creation`, defaulting it) or a struct literal, which
  does not exist outside `kuru-memory`'s own test/test-support code today.
  `grep -rn "OpenOptions {" packages/ apps/` (excluding `store.rs` itself) is
  the acceptance check named in the proposal; no new compile-time guard is
  added because the field is already unreachable from another crate without
  `test-support`, and `test-support` is not a default feature of
  `kuru-memory` (checked against `Cargo.toml`).

## Risks / Trade-offs

- **[Risk]** Splitting one function's control flow across a new module can
  silently change which errors get which `.context(...)` text if a
  `.context()` call is dropped or reworded during the move. **Mitigation**:
  each job's error paths (including the `(Ok, Err)`, `(Err, Ok)`, `(Err,
  Err)` match arms around `close_migration_worker`/`close_failed_open`) are
  moved verbatim, not rewritten; existing tests that assert on specific
  error strings (recovery, marker-boundary, and preservation tests) are the
  regression check, run unchanged.
- **[Risk]** The worker's job-oriented shape invites adding the template
  build job inside this change once the extraction exists, expanding scope.
  **Mitigation**: the proposal's non-goals and this design's non-goals both
  name the template job explicitly as out of scope; tasks below include no
  step that adds a fourth job.
- **[Risk]** Widening a field from `cfg(test)` to
  `cfg(any(test, feature = "test-support"))` changes what's compiled into a
  `test-support`-feature (non-`cfg(test)`) build of `kuru-memory`, which
  ships to `kuru-tui`'s fixture-using dev dependencies. **Mitigation**: the
  four existing `OpenOptions` test-only fields use plain `cfg(test)`, so
  `Creation` is the first field gated the wider way; this is deliberate per
  the proposal (test fixtures under `test-support`, not `#[cfg(test)]`, are
  `temporary_cold`'s actual callers from `kuru-tui` and other packages), and
  is checked by building `kuru-memory` with `--features test-support`
  without `--cfg test` in CI's existing test matrix.

## Seam ownership

- `store.rs` (`open_inner`) owns: the fresh-vs-existing-store branch
  decision, legacy-import preparation and `recover_staging`, calling the
  stage worker's three jobs in order, the move-to-active rename, and the
  active-path open (start 4) — unchanged.
- `store/stage_worker.rs` (new) owns: the `init`, `migrate`, and
  `validate-and-mark` job bodies, each including its own engine start,
  close, and `preserve_unready_stage` call on failure. It does not own
  `recover_staging`, the move-to-active rename, or any active-path logic.
- `store.rs` continues to own `run_migration_worker`, `close_migration_worker`,
  `close_failed_open`, and `preserve_unready_stage` as shared helpers called
  from both the stage worker and the existing-store behind-schema path; the
  stage worker calls them, it does not redefine them.
- `OpenOptions` (`store.rs`) owns the new `Creation` field and its default;
  `test_support.rs` and the `temporary()`/`temporary_cold()`/
  `open_temporary()` fixtures own setting it.
