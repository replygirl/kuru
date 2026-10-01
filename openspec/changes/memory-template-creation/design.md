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
