# Proposal

## Why

The store-creation design for copying new project stores from a pre-migrated
template (`tmp/roadmap/store-creation-design-2026-09-29.md`, decision (b) and
its dependent decisions (a)/(c)) rests on eight engine behaviors the pinned
Dolt 2.3.5 (upstream commit `ad65af6c`) has never been checked against in this
tree: root creation from the environment on a copied data directory, a single
bootstrap connection committing identity on two refs, branch dirty-state and
`AS OF` access without a branch pool, commit hash shape, and whether a copied
data directory holds no bytes that identify its origin. Decision (c)'s
proposed record-aware classifier also assumes every inline check
`classify_historical_attempts_in` runs today (`packages/kuru-memory/src/store/migrations.rs:1502-1573`)
has a main-pool equivalent; that assumption is untested. Establishing these
by test before design work continues means a negative result changes the
design instead of surfacing as a production defect.

## What Changes

- Add `packages/kuru-memory/src/store/engine_contract_tests.rs`, a new
  `#[cfg(test)]` module registered from `store.rs` alongside
  `open_pool_budget_tests`/`recovery_tests`/`template_tests`, holding one
  deterministic test per spike item against the real supervisor, server and
  pool code and the existing isolated fixtures:
  - S1: root creation from the environment on a data directory file-copied
    from a stopped, fully migrated store with an empty config directory; the
    copy's original users do not exist.
  - S2: one bootstrap connection commits an `UPDATE` of the instance row
    followed by `DOLT_COMMIT` on the usage branch and on main; each ref's
    head afterward differs from the template's and holds the new identity.
  - S3: whether the dirty state of a retained migration branch can be read
    from the main pool without opening a pool on that branch (`dolt_branches.dirty`
    or a revision-qualified `dolt_status`), stating which one works.
  - S4: the reader user can run `AS OF` queries against a retained branch and
    a commit hash from a connection on main.
  - S5: a test artefact capture format plus a consumer test that opens and
    adopts a checked-in or CI-provided cross-OS capture when present, and
    otherwise asserts an explicit not-run result. No capture is checked in by
    this change; no workflow file is added.
  - S6: commit hash width and format as the engine returns it, against what
    the record design (decision (c)) assumes.
  - S7: a byte scan of every file a stopped, copied store's data directory
    contains (including any `.dolt/stats` statistics store), asserting none
    holds the source directory's absolute path, host name, or either secret
    of the store it was copied from.
  - S8: for each inline check in `classify_historical_attempts_in` (head
    revision, schema version `AS OF`, dirty count, receipt match, sole
    parent, parent schema validation, ancestry of head and parent in main's
    log), whether an equivalent main-pool query exists and returns the same
    answer on a migrated store, including for a branch whose
    `kuru_instance` row differs from main's.
- Add test-support helpers only where an existing helper cannot produce the
  fixture (e.g., a stopped-store byte scan, a cross-OS capture reader), under
  `packages/kuru-memory/src/test_support/`, each exercised by the tests above.
- Update `docs/development.md` and `packages/kuru-memory` test registration
  only as needed to name the new module; no other doc changes.

## Impact

- `packages/kuru-memory/src/store.rs`: one new `#[path]`-registered test
  module.
- `packages/kuru-memory/src/store/engine_contract_tests.rs`: new file.
- `packages/kuru-memory/src/test_support/`: new test-support helper file(s)
  only, no product code.
- No product code, workflow file, dependency, or mise task changes. Adds
  wall-clock time to `mise run //packages/kuru-memory:test` (real Dolt
  subprocess starts per spike item); no new CI job.
