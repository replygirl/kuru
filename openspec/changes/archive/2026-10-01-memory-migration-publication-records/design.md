# Design

## Context

`packages/kuru-memory/src/store/migrations.rs` runs an immutable registry of
schema steps (`CURRENT_VERSION = 7` today). Each step builds an isolated
attempt branch (`build_attempt`), validates it (`validate_attempt`) and
fast-forwards it onto `main` (`publish`). Main-pool classification (already
shipped: `openspec/changes/archive/2026-09-30-main-pool-migration-classification`)
re-derives, on every open, whether each retained attempt branch is a
legitimately published historical step, entirely from the pool the caller
already holds — no branch or commit pool is opened — but it still re-derives
the verdict from scratch every time: branch head, `AS OF` schema/receipt
reads, sole-parent check, two ancestry lookups, per branch, per open.

The embedded store template
(`openspec/changes/archive/2026-10-01-memory-template-creation`) means every
new project is born with a fixed set of already-published main-registry
branches (one per step V2..V7) that will never change again, yet every open
of every project still re-derives their published status.

The maintainer's decision (store-creation design doc section 7, decision
(c)) is to record a step's publication, inside the attempt's own commit, so a
later open can verify the record against main's committed history — ref
hash, sole parent, ancestry, `AS OF` receipt and schema — instead of
re-deriving the full verdict. The record must never be the sole basis for
treating a branch as published: a branch with no record still gets full
classification, and a record that disagrees with main's actual history fails
the open closed.

Full research grounding: `tmp/roadmap/store-creation-design-2026-09-29.md`
section 7, `tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`
sections 7.5, 7.6 and 3.2, and
`tmp/roadmap/store-creation-research/C-migrations-and-D-dolt-upstream.md`.

## Goals / Non-Goals

**Goals:**

- A new schema step V8 (table `kuru_migration_publications`) that records
  every main-registry step's publication inside that step's own attempt
  commit, so the record and the schema/receipt reach main in the same
  fast-forward.
- V8's own attempt commit backfills one record per already-published branch
  (v2..v7), derived only from the full classification V8 has just run on that
  open — never from branch names.
- A later open verifies a recorded branch independently (never by chaining
  one record's base to another record's head) and fails closed on any
  disagreement, with no fallback to full classification for that branch.
- A branch with no record keeps today's full classification unchanged.
- The template-shape check and the schema-authority join both treat
  `kuru_migration_publications` as authority (exempt from the project-data
  zero-rows rule), so a template-born store's records are expected, not
  flagged.
- The one-time upgrade for stores below V8: one writable open does full
  classification once more, applies V8 with its backfill, reopens and
  validates; read-only opens are refused until that writable open, which is
  today's existing behavior for any pending step, not new behavior.

**Non-Goals:**

- No change to what a branch must satisfy to count as published (the
  classification rules themselves are unchanged; only how a later open
  re-confirms a previously-classified branch changes).
- No deletion or reclamation of retained branches.
- No change to the cold or template creation sequences beyond the new V8
  step executing like any other registry step.
- No change to timeouts, deadlines or retries.
- No change to the F2 upgrade reopen path, or to `store/stage_worker.rs` /
  `open_inner` (that is P5, developed in parallel on another branch; this
  change stays scoped to `migrations.rs`, the template shape check, the
  template key inputs and tests so that rebase stays mechanical).
- No change to the *shape* of the template-key mechanism (`compose()` in
  `creation_template.rs` stays one framed SHA-256 over compiled constants):
  `KeyInputs` gains one more scalar field (`record_format`), framed the same
  way `schema`/`usage_schema` already are. `CURRENT_VERSION` is already a key
  input, and V8 joining `DEFINITIONS` changes the key by that route alone;
  the explicit `record_format` field exists so a future record-format change
  with no accompanying schema bump still changes the key, matching the
  store-creation design's key-inputs table (section 3.2), which lists "the
  record format" as a distinct input alongside the schema versions and
  definition digests.

## Decisions

### Where the record lives and when it is written

The record lives in a new main-registry-only table, created by V8, with one
row per recorded step (`version` as primary key). It is written inside
`build_attempt`'s existing transaction, after the receipt `INSERT` and before
`DOLT_COMMIT`, for every step at or above `to = 8`. `build_attempt` gains
`branch: &str` and `base: &str` parameters (both already available to its
caller, `upgrade_in`, which already threads `base` and already has `branch`
from `discover_current_attempt_in`). This is the LC ("least change")
placement the store-creation design settled on: it needs no new crash-window
handling, because the record is just one more statement inside a transaction
whose commit-or-not is already reconciled by `build_attempt`'s existing lost-reply
logic (`validate_version_with` after a dropped `DOLT_COMMIT` reply), and
`publish`'s existing exact-base fast-forward already moves the whole commit —
record included — onto main atomically from main's point of view.

**Rejected alternative**: write the record as a separate commit on main
after the fast-forward. Rejected because it reopens the crash window the
maintainer explicitly closed off (decision (c) says "no crash window, no
direct commit on main") and because the step's own attempt commit already
carries exactly the evidence (branch, base, target, merge landed) that a safe
record requires per the C-migrations research note ("a record must be
written only at the moment publish() succeeds, from the evidence it already
holds").

### Backfill at V8

V8's own `build_attempt` call backfills v2..v7: `classify_historical_attempts_in`
/ `classify_retained_attempts` is refactored to optionally return the
`(name, head, parent)` of every branch it accepts as a clean completed
attempt (today it only returns `Result<()>`). `upgrade_in`'s V8 iteration
runs that classification first (as every iteration already does via
`classify_historical_attempts_in`), captures the accepted set, and passes it
to `build_attempt` so V8's own transaction inserts one row per accepted
branch, in addition to its own V8 row. `validate_attempt`'s V8 check verifies
the backfill rows equal the accepted set of the current classification; a
reused completed V8 attempt whose backfill disagrees fails closed without
mutation (mirrors `discover_current_attempt_in`'s existing reuse path for a
ready completed attempt).

**Rejected alternative**: backfill from branch names (parsing
`kuru_migration_v{to}_{uuid}`). Rejected: AGENTS.md's rule that "nothing is
ever classified as published from its name or existence alone" is exactly
the invariant decision (c) exists to preserve; backfilling from names would
let a crafted or corrupted ref look published without ever having been
checked.

### Verification shape on a later open

For each recorded branch, independently (never chaining one record's base to
another record's head, so no record's integrity depends on another record):
name parses and the record's own branch/operation/digest fields match the
compiled definition; `dolt_branches.dirty = 0`; exactly one parent via
`dolt_commit_ancestors`, equal to the recorded base; both head and base
appear in `dolt_log` as ancestors of main; one `AS OF` query against the head
reads `kuru_schema.version` and `kuru_migrations.operation`, both checked
against the record. All reads go through the pool the caller already holds;
no branch or commit pool opens. This is exactly the check the store-creation
design (section 7, point 3) specifies; this design makes no change to its
shape, only implements it.

A record whose named branch is missing from `dolt_branches` is accepted as
long as its `base` is still in main's history — this matches today's
behavior, where classification only iterates branches that exist; a deleted
retained branch was never itself a correctness requirement.

### Record-aware classification replaces today's per-branch classification, not the classifier's role

`classify_historical_attempts_in` (and its pool-level sibling
`classify_retained_attempts`) gains a record-aware fast path: for a branch
with a record, run the independent record check above instead of the
existing `AS OF`/receipt/sole-parent/ancestry sequence; for a branch with no
record, run exactly the sequence that exists today. The function's callers
(`validate_active`, `validate_inspection`, `validate_ready`, `upgrade_in`'s
loop) are unchanged — they already call this one function and get its
verdict.

### Authority join and template-shape check

`authority_working_set`'s dirty-authority check and `template_shape.rs`'s
`AUTHORITY_TABLES` both gain `kuru_migration_publications` alongside
`kuru_instance`, `kuru_migrations` and `kuru_schema`: a template or a store is
expected to have rows in this table, and a working change to it is an
authority violation, same as for the other three. The template-shape
derived-commit-count formulas (`BASE_COMMITS + REGISTRY.definitions.len()`
for `main`) need no edit: V8 is one more entry in `DEFINITIONS`, already
counted generically.

### Test-only V8 renumbered to V9

`kuru.memory.test-marker.v8` (`TEST_REGISTRY current: 8`) becomes
`kuru.memory.test-marker.v9` (`TEST_REGISTRY current: 9`), since V8 is now a
real reserved main-registry step. `TEST_DEFINITIONS` gains the real V8
ahead of the renumbered test step. Tests that assert on `TEST_REGISTRY`'s
`current` or on the test marker table/id update accordingly.

## Risks / Trade-offs

- **[Risk]** A bug in the record-write path could write a record for a step
  that did not actually reach the fast-forward (e.g. an aborted attempt whose
  transaction never commits). **Mitigation**: the record lives inside the
  same transaction and the same commit as the schema/receipt; if the
  transaction does not commit, neither does the record — there is no
  intermediate state where the record exists without the schema advance, by
  construction of a single Dolt commit.
- **[Risk]** The V8 backfill could record a branch that full classification
  should have rejected, if the refactored "accepted set" return value
  diverges from the classifier's own pass/fail logic. **Mitigation**:
  `classify_retained_attempts` already `ensure!`s on every rejection path
  before returning; the backfill set is built only from branches that reach
  the function's successful-return path, from the same code path (not a
  parallel re-implementation), and a test
  (`v8_backfills_every_published_branch_from_classification`) asserts the
  backfilled set is exactly what classification accepted.
- **[Risk]** A record could silently paper over a divergence between the
  record and main's actual history if verification is incomplete.
  **Mitigation**: verification checks ref hash, sole parent, ancestry in both
  directions and an `AS OF` receipt/schema read — the same four axes the
  store-creation design specifies — and any disagreement fails the open
  closed with no fallback, per the amended spec scenario.
- **[Risk]** The template key changes (V8 joins `DEFINITIONS`), so every
  build that has not yet refreshed its embedded template pays one rebuild.
  **Mitigation**: this is the existing, by-design behavior of the template
  key (it already changes whenever a registry definition changes); CI's
  `bundle-inputs` job and the release run already produce a fresh template
  per run, so this is a one-time cost absorbed by existing automation, not a
  new manual step.
