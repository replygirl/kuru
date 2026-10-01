# Design

## Context

Full design, measurements and review dispositions live at
`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md` (sections 3,
6, 7.2-7.6, 8-15); this file does not restate that content, only the
decisions this PR (P4b-i) commits to and why, and where this PR's scope ends.

PR #145 (`memory-template-stage-adoption`, archived) already landed the
receiving side with no caller: the `identity.json` `template` field, bootstrap
adoption gated on a compiled-key comparison against a placeholder constant,
the typed `Response::TemplateRejected` verdict, the shared
`store/migrations/template_shape.rs` check (parameterized by placeholder vs.
adopted identity), and recovery classes R (copy remnant, no identity) and U
(unready template stage). None of that is changed here. This change supplies
what #145 had nothing to receive from: the cache a template lives in, the key
that names it, the build that produces it, the verified copy primitive, and
the CI warm-up that proves all of it once per job before any open depends on
it.

Constraints carried over from the roadmap design and from AGENTS.md: the
owned supervisor still reaps Dolt before releasing its directory or lifecycle
lease; build-input mirrors (the bundled engine archive) stay separate from
this runtime cache; private-object validation is never weakened to accept a
restored directory; coverage's instrumented-fixture rule applies to the
template build the same way it applies to the engine; and no new environment
variable is introduced — the existing `cache_dir`/`KURU_DOLT_CACHE` mechanism
is reused for the template root.

## Goals / Non-Goals

**Goals:**

- Land every piece of the template cache (layout, key, build, capture,
  publication, verification, quarantine, no-GC) as library code with its own
  tests, reachable from `ensure_in`/`create_in` with the template root as a
  parameter, so P4b-ii can wire a product open to it without re-deriving any
  of these rules.
- Land CI fixture warm-up (engine then template, per job, every OS, under
  coverage) and the deterministic unwarmed-fixture guard, so a missed warm-up
  call site fails loudly in this PR's own CI run rather than silently
  degrading P4b-ii's fixtures later.
- Keep every existing product start count, deadline and retry unchanged,
  because nothing reads the template on a product open path yet.

**Non-Goals (deferred to P4b-ii unless noted):**

- Routing `Creation::Default` through the template, the copy worker inside
  `open_inner`, and cold fallback on a template failure.
- Restart removal (P5) and the migration-publication records table (P6);
  `BASE_COMMITS` and the branch/commit-count formulas these PRs would add stay
  as `template_shape.rs` already has them.
- A reclamation command for old template keys (recorded as a possible
  follow-on in the roadmap design, section 3.8 item 6; not here).
- Changing the product's `fresh_open_budget`/`fixture_deadline` multipliers —
  those change only once a product open actually takes the template path.

## Decisions

- **Template root as an explicit parameter, not a derived path.** `ensure_in`
  and `create_in` take the template root directly; product code passes
  `<cache>/<DOLT_VERSION>/templates`, and tests pass a private root through a
  new crate-private `OpenOptions` field under `cfg(any(test, feature =
  "test-support"))`. Rejected alternative: deriving the root from
  `config.cache_dir` inside the module itself, which would force every test
  needing an empty or damaged template root to also fake a whole cache
  directory and would make the shared test cache's warm template visible to
  tests that must not disturb it (T2, T3, T5-T8, T13, T14, T17, T20-T22).
- **One shape-assertion function, shared by build and by S1 adoption.**
  Already decided by #145 for the copy side; this PR's build calls the same
  function with the placeholder identity. Rejected alternative: a separate,
  simpler build-time assertion — rejected because a bug in a second function
  could pass a build that the shared function would then also pass at S1,
  defeating the point of catching a check bug at the build instead of at
  every later copy (roadmap design 3.3).
- **Key excludes supervisor bytes and source digests.** Keeping an
  instrumented and an ordinary supervisor's templates interchangeable is
  required by the coverage rule; the key is restricted to the engine and
  statement inputs that provably shape an *empty* store's bytes (the
  migration chain's Rust transform code only acts on existing rows, and the
  template has none). Risk and guard: a rule-plus-unit-test enumerating every
  keyed input, so a new creation-path constant added without being keyed is
  caught at review time rather than producing a stale template silently
  (roadmap design 13, "Key completeness without a source digest").
- **Quarantine only on a positive verdict against bytes, identity-bound.**
  Matches blocking finding B1's resolution (roadmap design 15.1): an engine
  failure, deadline, lock contention or I/O error never touches a template.
  The quarantine action re-checks the judged directory's identity under the
  exclusive lock immediately before the rename, so a template republished by
  another process between the verdict and the quarantine attempt is never
  condemned.
- **No garbage collection on any open path; lock files permanent.** Matches
  B2's resolution. Rejected alternative: pruning other keys' templates or
  stale lock files opportunistically on open — rejected because the shared
  test cache is one directory per machine used by every worktree
  concurrently, and GC racing against a concurrent build or copy in another
  worktree is exactly the failure B2 found. A maintenance command for user
  machines is recorded as a future option, not built here.
- **Warm-up order and the per-fixture token, not a global "is it warm"
  flag.** Matches B3's resolution. `temporary()`/`temporary_cold()` warm
  before `instantiate`/the permit; a crate-private `Fixture` token on
  `OpenOptions`, set only by `warmed_open_options`/`OpenOptions::warmed()`,
  makes the guard in `open_inner` depend on the call site's own history, not
  on process-wide state, so it fires deterministically on the first run of
  every missed call site regardless of test order or what else already
  warmed the shared cache.
- **Warm-up never starts under a held `spawn_gate::spawning()` guard.** The
  gate is a fair `RwLock`; a first warm-up running under a caller's own read
  guard while a `locking()` writer is queued would deadlock a nested read
  behind that writer. This PR's own audit of every `spawning()` site that
  encloses an open (service.rs, facade.rs, operational_gc_tests.rs) is part
  of the acceptance evidence, not assumed from the roadmap design's one-time
  count.

## Risks / Trade-offs

- [Key correctness without a source digest] → mitigated by T10 parity against
  a cold store, the keyed-input enumeration test, and validation/shape checks
  on every copy that catch schema and history drift even though they cannot
  catch every possible bytes-changing code change between releases.
- [Copy cost and per-file verification overhead, especially many small files
  under Windows real-time scanning] → unmeasured until this PR's own M0-style
  local measurement; recorded as acceptance evidence rather than assumed from
  the roadmap design's baseline, which was taken from a different (test)
  template's manifest.
- [Spawn-gate deadlock if the audit misses a site] → mitigated by making the
  audit itself a required, recorded piece of evidence (every `spawning()`
  site enclosing an open, not just the ones already touched), and by the
  deterministic guard failing any missed warm-up site the first time CI runs
  it.
- [A rename-publication failure leaves a verified stage the build itself must
  reuse] → the stage is left in place for the next exclusive-lock holder to
  resolve (sweep-on-build-entry already covers it); this PR tests the
  failure-to-leave-stage path without a caller yet using it, since no copy
  path exists until P4b-ii.
- [Whether `apps/kuru-tui/src/ui/runtime_tests.rs` calls `cache_dir()` inside
  a Tokio runtime is unknown until this PR makes the accessor fallible] →
  resolved by this PR's own change to that call site, recorded as evidence
  rather than assumed.
