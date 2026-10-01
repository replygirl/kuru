# Design

## Context

`packages/kuru-memory` already has a per-machine store template cache
(`store/creation_template.rs`: `ensure_in`, `create_in`, `copy_into`,
`quarantine`, `CreationFailure`), a stage worker (`store/stage_worker.rs`:
`StageWorker`, the `TemplateBuild` job, `adopt_and_mark`), an adoption
bootstrap in `server.rs` with `Response::TemplateRejected`, and the
`recover_staging` classes for a copy remnant and an unready template stage —
all merged and durably specified in `versioned-memory`. None of it is
reachable from an ordinary open yet: `OpenOptions::Creation::Default`'s own
doc comment says it "takes today's cold staged build," the same as
`Creation::Cold`.

This change is the wiring PR (P4b-ii in the store-creation design at
`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`, read in
full before this change was authored): the path selector in `open_inner`
after `recover_staging`, the first-project build-then-copy ordering, the
cold fallbacks, the fixture class guards and the budget updates. Full
mechanics, the lock ownership table, the crash/cancellation table and every
numbered step are in that roadmap document (sections 2.1–2.5, 3.4, 3.7–3.9,
5); this file records only the decisions this change's own tasks depend on,
not a restatement of that design.

## Goals / Non-Goals

**Goals:** see the proposal's "What Changes." In short: wire the selector,
the build-then-copy ordering for a machine's first project under a key, the
already-specified cold fallbacks, and the test-support budgets and class
guards that make the new start counts observable and regression-tested.

**Non-Goals:**
- No restart removal on the cold path (a separate `perf` change); the cold
  path stays at today's start count.
- No records/receipts work (a separate `feat` change).
- No reclamation command, no new environment variable, no change to the test
  cache mechanism, timeouts, deadlines or retries.
- No change to an existing project's open path.

## Operational surface

The interactive surface this change touches is the existing command-line
activity sentence (`apps/kuru-tui/src/memory_activity.rs`, landed in #148):
this change adds no new sentence, stage, bind address, container/runner
topology, required secret or connection limit. It keeps the build-then-copy
and warm-copy creation paths inside the already-reported `CreatingDatabase`
stage so the existing S3 sentence ("Creating this project's memory…") covers
both without a wording or UI change. The creation worker starts the same
owned, pinned full-Dolt binary the cold path already starts (two or three
times instead of four), using the same per-project writer lease and startup
lock; no new listener, process topology or binary version is introduced.

## Decisions

- **Selector runs once, right after `recover_staging`, never mid-stage.**
  Rejected: re-evaluating eligibility after a stage already exists, which
  would let a project flip paths mid-open and complicate recovery
  classification. The roadmap's table 2.1 is authoritative for the exact
  condition order (legacy import, configured binary, test-only `Cold`,
  published template, unpublished template, lock/structural error).

- **The creation worker, not the opener's own frame, holds the startup lock
  and the template key lock while a copy or build runs.** This reuses the
  `run_migration_worker` ownership shape already proven for the
  migration-chain stage worker: the same discipline avoids a cancelled
  opener abandoning a held lock mid-copy or mid-build. Rejected: holding the
  locks in the opener's frame and handing them to a spawned task only for
  the blocking copy, which reintroduces the cancel-unsafety the existing
  worker shape was built to avoid (the roadmap's RG D1 disposition).

- **First project on a fresh machine builds, then copies, in one open; a
  second concurrent project never waits.** Rejected alternatives (roadmap
  2.3, options A1–A3): cold-then-build-in-the-same-open pays the
  schema-migration chain twice; building on the next new project, or in the
  background after `Ready`, either pays the chain twice across two projects
  or does engine work after the client's own open returned. "Build then
  copy" pays the chain exactly once and gives both the first and later
  projects a bounded, deterministic start count.

- **Cold fallback is silent to the caller.** A busy lock, a lock error or a
  structural verdict against a template must never surface as a
  template-specific failure on an ordinary new-project open: the open
  completes on the cold path as if no template had ever existed, matching
  the template cache requirement's own "no usable template for this key"
  rule. Surfacing these as visible errors here would turn an internal cache
  optimization into an externally observable new project-creation failure
  mode, which the maintainer's acceptance criteria (no regression on
  existing-project open; new-project open not worse than today on a cold
  cache) rule out.

- **No in-open retry after a verdict or an engine-side failure on the
  template path.** Already decided for the underlying adoption/shape
  protocol; this change's selector simply returns the resulting error (for a
  verdict or adoption/shape failure) or falls back to cold (for the
  eligibility cases above) rather than attempting a second template copy or
  build inside the same open.

- **Failure mapping in the creation worker.** `creation_template::create_in`
  already carries the discriminant (`CreationFailure::{TemplateVerdict, Engine,
  Io}`); the worker maps it without a second classification:
  `Unavailable` (busy or lock error), `TemplateVerdict` and `Io` go cold in a
  new stage, after preserving any copy remnant (Class R) and removing an empty
  stage; `Engine` (only the template build's engine, SQL or capture can
  produce it) returns its error. A shape verdict on the *build* engine is
  classified `TemplateVerdict` with nothing published, so it also goes cold:
  the cold path never runs the shared shape check, so the user's open
  succeeds, and the next new project tries the build again. Rejected:
  returning that error, which would turn a build-side check bug into a
  new-project failure on every machine.

- **The copy runs on a blocking thread that holds the key lock.** `create_in`
  hands each copy, with the key lock, to `spawn_blocking` through a second
  handle on the stage bound to the stage's identity, and gets the lock back
  with the result; the creation worker's task owns the startup lock. Test
  hooks are task-local, so the worker and the copy re-enter the caller's hooks
  explicitly.

- **A failure of the copied stage's own engine start leaves the stage where it
  is.** Adoption runs in the start's bootstrap, so the startup lock was the
  failing server's reap guard and is released after the reap; the worker does
  not move the stage without it. The next open's recovery preserves it as
  Class U without an engine start, as the adoption requirement already says. A
  failure after the start (validation, shape, `ready.json`) is preserved by
  the staging job itself under the returned lock. A verdict in either case
  quarantines the judged template through `quarantine_after_adoption`, which
  takes the key's exclusive lock without waiting and needs no startup lock.

- **Fixture budgets keep four starts per fresh open.** `fresh_open_budget_of`
  budgets each path (`FreshOpen::{Template, FirstProject, Cold}`), but the
  default fresh-open budget and `fixture_deadline` keep the cold build's four
  starts: an open can still fall back to it until restart removal makes the
  cold path two starts. No deadline changes.

- **The whole template path runs inside the already-reported `CreatingDatabase`
  stage; no new sentence or stage is added.** `open_inner`'s `!Self::exists`
  branch reports `MemoryOpenStage::CreatingDatabase`
  (`packages/kuru-memory/src/store.rs:1890`) before `recover_staging`, before
  `creation_worker::select` is even called (`store.rs:1917`), and before the
  creation worker's `OpeningDatabase` report inside the template branch
  (`store.rs:1921`). Under #148's stage-mapping decision D3
  (`openspec/changes/archive/2026-09-30-memory-open-activity/design.md`), S3
  ("Creating this project's memory…") is shown on `CreatingDatabase` (rule
  R5) and *kept* on the following `OpeningDatabase` report (rule R7, "keep").
  So the build-then-copy case (three engine starts: the build, the copy's
  adoption start, the active start) and the warm-copy case (two starts: the
  copy's adoption start, the active start) both run entirely after S3 is
  shown and before any stage that would change or clear it. This satisfies
  #148's own risk note ("Conflict with the template-copy change in
  `open_inner`") without touching `memory_activity.rs`'s stage sites or
  sentence table. Rejected: a new `BuildingTemplate` stage or sentence
  distinguishing the build from an ordinary creation, which the maintainer
  instruction for this change rules out (no new sentence, stage name or
  wording change without agreeing it with assistant4, which this change does
  not do) and which the existing mapping does not need — S3 is already true
  for "this project's database is being created," including by a
  machine-first template build.

- **Test coverage reuses #148's hold hook and PTY fixture; one new test, one
  confirmed-sufficient existing test.** `real_pty_accepts_chat_navigation_commands_and_restores_terminal`
  (`apps/kuru-tui/tests/terminal.rs`, via its `smoke(..., expect_notice: true)`
  helper) already runs against `Sandbox::new()`, which warms the shared
  engine *and* template cache synchronously
  (`test_support::cache_dir` → `warm_runtime_cache` →
  `warm_template_in`), holds the open at `CreatingDatabase` with
  `KURU_TEST_MEMORY_OPEN_HOLD_DIR`/`CreatingDatabase.hold`, and asserts the
  `CREATING` sentence is on the terminal before releasing the hold — this is
  already the warm-template copy-and-adoption case for this change's
  activity-sentence goal; it needs no new test, only re-running after this
  change lands to confirm it still exercises the two-start path (not a
  rewrite of the test). The build-then-copy (first launch, empty template
  cache) case has no existing coverage: no PTY test in
  `apps/kuru-tui/tests/terminal.rs` opens against an unwarmed, private cache
  directory. This change adds one: a private `tempdir`-backed cache (not
  `test_support::cache_dir()`'s shared warmed one), the same
  `CreatingDatabase.hold` mechanism, and the same assertion that `CREATING`
  is on the terminal while the hold is held, synchronized on a completed
  frame per `AGENTS.md`; it releases the hold and lets the build-then-copy
  path (three starts) finish to a ready frame.

## Risks / Trade-offs

- **[Risk] The selector regresses an existing project's open path** by
  touching `open_inner` near `recover_staging`. → Mitigation: the selector
  is gated strictly on "active store absent"; existing-project opens never
  reach it, and `store/open_pool_budget_tests.rs` and the existing
  recovery/migration test suites continue to assert the unchanged path
  unconditionally.

- **[Risk] A missed cold-fallback case turns an internal cache condition
  into a visible open failure.** → Mitigation: tasks enumerate every
  fallback condition from the already-landed template-cache requirement, and
  T15/T20/T22-equivalent tests (named in the roadmap's section 9, scoped to
  this change) assert each one completes as an ordinary cold-path open with
  no template-specific error surfaced.

- **[Risk] Test fixtures silently build a template inside a timed-fixture
  open**, now that ordinary opens can do so for the first time. → Mitigation:
  the fixture class guard and warm-up ordering already landed in the
  template-cache change; this change's own tests extend the guard's
  coverage to the new build-then-copy and warm-copy paths so a fixture that
  reaches either unwarmed still fails fast instead of masking a slow first
  open.

- **[Risk] A future stage added to the template path (a new copy sub-stage,
  a second adoption attempt) lands between `CreatingDatabase` and the next
  stage this change's selector reports, and D3's rule R9 ("any other or
  future stage: keep") means it would silently keep S3 even if it should
  not.** → Mitigation: none needed for this change (it adds no new stage);
  recorded so a later change that adds a stage inside the template path
  re-reads #148's D3 table rather than assuming R9 is always correct for a
  stage it did not evaluate.
