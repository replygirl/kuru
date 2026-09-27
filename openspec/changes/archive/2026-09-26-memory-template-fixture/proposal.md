## Why

Every test that opens a temporary Dolt-backed `MemoryStore` (about 205 sites:
roughly 140 in `kuru-runtime`, 60 in `kuru-memory`, 5 in `kuru-tui`) pays a full
cold open. Measured locally at about 4.2 s, about 60% of which is migrations v1
to v7 and two `validate_active` passes; hosted CI opens take 9 to 12 s. A copied,
already migrated template directory reopened through the unchanged existing-store
path measured about 0.67 s, with every test still owning its own writable open,
supervisor and Dolt process.

## What Changes

- `packages/kuru-memory/src/test_support.rs` (and a new
  `packages/kuru-memory/src/test_support/template.rs` if it grows): create one
  pre-migrated template data directory per fingerprint on disk, in the private
  per-profile test directory beside the prepared supervisor snapshot. The
  fingerprint hashes the supervisor that `test_supervisor()` actually returns
  (the prepared snapshot, or under coverage the instrumented helper), the
  current schema versions, the pinned engine version, the fixture scope, a
  capture format constant and the sources that define the stored schema
  (migrations run in the test process, not the supervisor). Creation holds a file lock, stages in a private
  sibling `<fingerprint>.stage-<uuid>` under the template root, captures only a
  cleanly closed cold open (after reap and lifecycle-lease release) and
  publishes by moving the verified stage to its final name. Reuse revalidates the
  private objects; a template that fails validation is rebuilt under the lock,
  never accepted. Correct for process-per-test runners; an in-process `OnceLock`
  alone is not relied on.
- Per-test copy through checked `kuru_platform::fs::Directory` handles
  (`Directory::create_private_directory` and `create_new` for each object) with
  an explicit allowlist of files. Lock, lease, PID, socket and endpoint files, including any
  Dolt server lock/info files left in `data/`, are never copied. Links are
  rejected; owner-only modes and Windows DACLs are created fresh, not inherited.
- `MemoryStore::temporary()` in `packages/kuru-memory/src/store.rs` is compiled
  only under `cfg(any(test, feature = "test-support"))`. It becomes
  template-backed by default, and an explicit cold opt-out (for example
  `MemoryStore::temporary_cold()`) keeps the full staged open. `open_inner` and
  every production path stay unchanged. If a test-only seam in product code
  proves unavoidable, work stops and is reported instead.
- Class-2 (server or process lifecycle), class-3 (migration, import, legacy) and
  identity, secret or migration-asserting `temporary()` callers in
  `packages/kuru-memory` move to the cold API. An explicit test in `kuru-memory`
  proves that the cold path still opens, beyond template creation itself.
- `kuru-runtime` and `apps/kuru-tui` reach the fixture through the `test-support`
  feature. Their `temporary()` sites are verified by suite runs and by a scan for
  cross-store identity or secret comparisons. Edits are expected only where such
  a comparison exists.

## Impact

Test support only: `packages/kuru-memory/src/test_support*.rs`, the
`test-support`-gated `temporary()` in `packages/kuru-memory/src/store.rs`, and
cold opt-outs at tagged test sites. No dependencies, product behavior, coverage
exclusions or thresholds change. Copies share the template's instance identity
and secrets, so identity-sensitive tests stay cold. Template creation runs once per
fingerprint per target directory, so an existing template means a binary may run
no template-creating cold open at all. Cold-path coverage comes from
`kuru-memory`'s `temporary_cold()` tests
(`cold_constructor_runs_every_migration_under_a_new_identity` and the migration
and receipt-authority tests) and from caller-owned `MemoryStore::open` sites. The expected local saving is about 2.8 s per open serially
(about 5 minutes for the `kuru-runtime` suite at `RUST_TEST_THREADS=2`), and more
on CI; the PR's native CI run measures the per-binary saving on each OS.
