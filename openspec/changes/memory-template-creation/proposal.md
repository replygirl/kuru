# Proposal

## Why

The per-machine store template cache (`creation_template.rs`, `stage_worker.rs`,
the adoption bootstrap and `Response::TemplateRejected`) and the template-born
stage identity/adoption protocol already exist and are durably specified in
`versioned-memory`, but no ordinary open uses them yet: `OpenOptions`'s
`Creation::Default` still takes today's full migration-chain cold build for
every new project, as its own doc comment states. New-project creation is
therefore paying the full schema chain on every open even when a verified
template for the current key is already published and warm.

This change wires the actual creation path: `open_inner` gains the selector
that chooses a verified template copy over the migration chain for an
ordinary new project, builds the template first (once per machine and key)
when none is published yet, and falls back to the cold path for every case
the template protocol itself already excludes (legacy import, a configured
`dolt_binary`, a busy or damaged template, a key mismatch). A new project on
a machine with a warm engine and template drops from 4 engine starts to 2;
the first project on a fresh machine pays the chain once, in 3 starts, instead
of today's 4. No restart removal, no records and no change to timeouts,
deadlines or retries are part of this change.

## What Changes

- Add the open-time path selector (`open_inner`, after `recover_staging`):
  cold for a legacy import, a configured `dolt_binary`, or test-support
  `Creation::Cold`; otherwise copy from a published template under a shared
  key lock, or build the template first under an exclusive key lock when none
  is published, then copy.
- Add the creation worker that owns the project's startup lock and the
  template key lock while the copy (or build-then-copy) runs, per the
  existing stage-worker ownership shape, so no opener frame holds a lock
  while work is in flight.
- Add the "first project on a fresh machine" path: build the template on one
  engine under the key's exclusive lock, publish it, then copy the new
  project from the published template (or from the verified build stage if
  publication fails), before the unchanged adoption, validation, rename and
  active start.
- Wire the cold fallbacks already specified for the template cache and
  adoption protocol (busy lock, lock-file error, structural verdict, I/O
  error mid-copy) into this selector, so an ordinary opener never waits on
  another process's template build or another project's startup lock.
- Extend test-support fresh-open budgets and the fixture class guard for the
  new start counts, and add the P4b-ii tests naming the selector, the
  two-starts and three-starts cases, and two concurrent new projects never
  waiting on each other.
- Update `docs/memory.md`, `apps/kuru-docs/concepts/memory.md` and
  `docs/development.md` to describe where new projects now come from, what
  the first launch on a machine pays, and the fixture start-count change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `versioned-memory`: adds the store-creation path-selection requirement
  (which path an ordinary new-project open takes, and under which locks) on
  top of the already-specified template cache and adoption protocol, which
  this change is the first to actually invoke from an ordinary open.

## Impact

- `packages/kuru-memory/src/store.rs`: the path selector in `open_inner`,
  removal of `Creation::Default`'s current "always cold" behavior for a path
  with a usable template, test-support budget and fixture-guard updates.
- `packages/kuru-memory/src/store/creation_template.rs`: `ensure_in`,
  `create_in`, `copy_into`, `quarantine`, `CreationFailure` consumed from an
  ordinary open instead of only from test fixtures and warm-up.
- `packages/kuru-memory/src/store/stage_worker.rs`: the `StageWorker`,
  `TemplateBuild` job and `adopt_and_mark` reused for the first-project
  build-then-copy path.
- `packages/kuru-memory/src/server.rs`: no new adoption wiring (already
  present); the bootstrap and `Response::TemplateRejected` are now reachable
  from an ordinary project open, not only from template/adoption tests.
- `packages/kuru-memory/src/test_support.rs` and its fresh-open budget
  helpers: new start-count cases for the warm-template and
  build-then-copy paths.
- `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`,
  `docs/development.md`: creation-path and fixture-budget documentation.
- No schema change, no new environment variable, no change to timeouts,
  deadlines, retries or the cold path's own start count (still 4, pending the
  separate restart-removal change).

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
