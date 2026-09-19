## Why

`supervisor_rejects_bad_configuration_and_parent_eof_without_spawning` in
`packages/kuru-memory/src/server_tests.rs` intermittently fails its final
assertion (`"not been initialized"` does not appear in the observed error,
which is `"... parent closed ..."` instead), even though nothing else in the
test touches its own `lifecycle.lock`. The cause is an operating-system
property of `flock`, not a defect in the assertion: `flock` ownership belongs
to an open file description, and `posix_spawn` copies the parent's whole
descriptor table into the child before image activation closes
`FD_CLOEXEC` descriptors, so a concurrently spawning thread transiently holds
a duplicate of a lock file's open file description. When the parent releases
its own descriptor inside that window, the lock stays held and the test's own
third `supervise()` call observes a still-contended lock and falls back to
the same "parent closed" bail as its second call, instead of reaching
`"memory has not been initialized"`. `cargo test -p kuru-memory --all-targets
--all-features` runs this lock-taking test in parallel with real-Dolt store
opens and process spawns from `provision/native_tests.rs`,
`store/recovery_tests.rs`, `store/migration_lifecycle_tests.rs`,
`store/operational_gc_tests.rs` and `server_tests.rs`'s own fixtures, which
supplies exactly that concurrency. This is the same class of hazard already
diagnosed and fixed for `apps/kuru-tui` in
`openspec/changes/archive/2026-09-18-serialize-test-lock-and-spawn`.

## What Changes

Add a crate-internal, test-only gate in `packages/kuru-memory` that
serialises advisory-lock acquisition/release against in-binary process
spawning: tests (and the small number of shared test-only fixture helpers)
that take a lifecycle, startup or cache lock hold the gate exclusively around
the actual `flock` acquisition/release, tests that spawn a child process (or
drive the crate's own `MemoryStore::open`/`Server::open`/`crate::engine::spawn`
in-process) hold it shared across the spawn, so spawning tests still run
concurrently with one another. No production code path changes: `server.rs`,
`engine.rs` and `provision.rs`'s own locking/spawning logic are untouched; the
gate is taken only by test code and by two new `#[cfg(test)]` test-support
helpers (`crate::test_support::spawn_gated_open`,
`provision/native_tests.rs`'s `gated_cache_lock`/`gated_verify_version`) that
give the exhaustive real-Dolt fixtures in `store/recovery_tests.rs`,
`store/migration_lifecycle_tests.rs`, `store/operational_gc_tests.rs` and
`provision/native_tests.rs` a single choke point instead of requiring an edit
at each of their dozens of individual call sites. The previously bare final
assertion in `supervisor_rejects_bad_configuration_and_parent_eof_without_spawning`
also gains a `{error:#}` diagnostic, matching its sibling assertion two lines
above it. No assertion is weakened, no timeout widened, no test retried or
ignored, and the tested fail-closed behavior is unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

New `#[cfg(test)]` module `packages/kuru-memory/src/spawn_gate.rs` (declared
in `lib.rs`) with its own regression tests. Test-only gate usage added in
`server_tests.rs` (every advisory-lock and process-spawning test/helper),
`store/migration_lifecycle_tests.rs` (the one test that holds a raw lock
across a real `Server::open_with_guard` spawn), and
`provision/native_tests.rs` (new `gated_cache_lock`/`gated_verify_version`
helpers). A new `#[cfg(test)]` helper `crate::test_support::spawn_gated_open`
in `test_support.rs` replaces the ~34 direct `MemoryStore::open(` call sites
in `store/recovery_tests.rs`, `store/migration_lifecycle_tests.rs` and
`store/operational_gc_tests.rs` with a single gated choke point. No public
API, configuration, capability spec or shipped behavior changes; the gate
compiles only under `cfg(test)` and never reaches a shipped binary,
installation, packaging or documentation surface.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
