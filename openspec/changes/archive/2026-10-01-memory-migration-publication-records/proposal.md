# Proposal

## Why

Every open that reaches `upgrade_in`'s classification loop or any of
`validate_active`, `validate_inspection` or `validate_ready` re-classifies
every retained migration attempt branch from scratch: `AS OF` reads, a
receipt match, a sole-parent check and two ancestry lookups per branch, over
and over, for branches that were already published and have not moved since.
Main-pool classification (already landed) removed the branch/commit pools
from that work, but the per-open cost of re-deriving "this branch is
historical" from first principles on every retained branch remains, and nothing
on disk distinguishes a branch that was checked and found published from one
that has never been checked. The maintainer has ruled (decision (c) of the
store-creation design) that a migration branch that was validated and
published should be recorded as such, so a later open can verify the record
against main's committed history instead of re-deriving the verdict — while
keeping the invariant that nothing is ever treated as published from its name
or existence alone.

## What Changes

- Add schema step V8 (`CURRENT_VERSION` 7 to 8): a new main-registry step
  creates table `kuru_migration_publications` (`version`, `branch`, `base`,
  `operation`, `definition_digest`, `record_format`) and, inside its own
  attempt commit, backfills one record per already-published branch (v2..v7)
  from the full classification it has just run on that open — never from
  branch names.
- Every main-prefix step at or above the publication-record version now
  writes its own publication row inside the same clean transaction as its
  schema DDL and receipt insert, before `DOLT_COMMIT`, so the record reaches
  main only through the existing exact-base fast-forward publish. There is no
  window between a step's own publication and its record: both ride the same
  commit and the same `DOLT_MERGE --ff-only`.
- A later open verifies each recorded branch independently from the pool the
  caller already holds (no branch or commit pool is opened): the record does
  not store a head (it cannot live inside its own commit), so the branch's
  own live head is read from `dolt_branches`, its sole parent equals the
  recorded base, both head and base are ancestors of main, and an `AS OF`
  read of the head matches the record's version and receipt operation. Any
  mismatch — a moved ref, a dirty recorded branch, a record whose branch is
  outside main's history, or a digest/operation mismatch — fails the open
  closed, with no fallback to full classification and no mutation. A branch
  with no record still gets today's
  full classification.
- `validate_attempt` checks the in-progress attempt's own row the same way,
  and the schema-authority join (`authority_working_set` /
  `AUTHORITY_TABLES`) adds `kuru_migration_publications` to the tables the
  template shape check exempts from the zero-rows rule, so a template-era
  store's records are expected, not flagged as stray data.
- The template-shape check (`store/migrations/template_shape.rs`) accepts and
  verifies records on template-era branches, and the template key (already
  keyed on `CURRENT_VERSION`, `USAGE_CURRENT_VERSION` and the digest of every
  registry definition) changes with V8 by construction, since V8 is a new
  registry definition — the key formula itself is unchanged. The first new
  project built after this change therefore rebuilds the embedded template
  once and pays the full chain once for that rebuild; every project created
  from the new template is born with records and never pays the one-time
  upgrade below.
- Existing (pre-V8) stores pay a one-time writable-open upgrade: the worker
  runs full classification one last time (today's behavior), then the V8 step
  with its backfill, then reopens and validates. A read-only open of a store
  behind schema continues to be refused with the existing
  "requires writable upgrade" message until that writable open happens — this
  is today's behavior for every pending step and is unchanged by this
  proposal.
- Rename the test-only V8 migration fixture (`kuru.memory.test-marker.v8`,
  `TEST_REGISTRY current: 8`) to V9, since V8 is now a real, reserved,
  main-registry schema step.

## Capabilities

### New Capabilities

(none — this extends the existing versioned-memory capability)

### Modified Capabilities

- `versioned-memory`: migration branch classification gains a durable
  publication record that a later open verifies instead of re-deriving, with
  the one-time V8 upgrade and backfill, and fails closed on any recorded
  branch whose ref, ancestry or recorded content disagrees with main.

## Impact

- `packages/kuru-memory/src/store/migrations.rs`: `REGISTRY`/`DEFINITIONS`
  (new `V8` definition), `build_attempt` (gains branch/base parameters and
  writes the publication row inside the attempt transaction for
  `to >= 8`), `publish` (unchanged mechanically — the record rides the
  existing fast-forward), `classify_historical_attempts_in` /
  `classify_retained_attempts` (record-aware: a recorded branch is verified
  by its record; an unrecorded branch keeps full classification),
  `validate_attempt` (checks the attempt's own record for `to >= 8`),
  `authority_working_set` / `AUTHORITY_TABLES` (add
  `kuru_migration_publications`), `upgrade_in` (passes the backfill set to
  V8's `build_attempt`), the test-only `V8` fixture renamed to `V9` and its
  `TEST_REGISTRY`/tests updated.
- `packages/kuru-memory/src/store/migrations/template_shape.rs`: the shape
  check accepts and verifies publication records on template-era branches;
  `AUTHORITY_TABLES` additions are shared with the schema-authority join.
- `packages/kuru-memory/src/store/creation_template.rs`: `KeyInputs` gains a
  `record_format` field, framed into `compose()`'s hash alongside `schema`
  and `usage_schema`, so the key tracks the record format as its own input
  (not only indirectly through `CURRENT_VERSION`, per the store-creation
  design's key-inputs table); the key already changes with V8 regardless,
  since `CURRENT_VERSION` is one of its existing inputs. The template build
  now produces a store carrying V8 and its records; the build assertions'
  derived commit counts (`BASE` + `REGISTRY.definitions.len()` for `main`)
  pick up V8 automatically.
- `packages/kuru-memory/src/store/usage_ledger.rs`: no schema change (records
  are main-registry only; usage-prefixed attempts keep full classification,
  and a template-born store has no usage-prefixed attempts by producer
  assertion) — confirmed, not modified, during implementation.
- `packages/kuru-memory/src/server.rs`: no change to `CREATION_STATEMENTS`;
  referenced only because the template key and shape check depend on it.
- `openspec/specs/versioned-memory/spec.md`: amend "Retained attempts outlive
  their migration step" (the scenario at the committed requirement
  "Isolated schema construction and publication") so a recorded branch is
  verified by its record and an unrecorded one is classified as today; add a
  scenario for a record mismatch failing closed and a scenario for the V8
  backfill upgrade.
- `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`: document the
  one-time writable-open upgrade and the existing read-only refusal message
  a user sees first.
- `docs/development.md`: fixtures are at schema 8; the embedded template
  rebuilds once for the new key.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
