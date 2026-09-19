## Why

`held_store_lock_and_in_workspace_storage_fail_closed` in `apps/kuru-tui/src/permission_store.rs` intermittently fails at its own `lock(&directory, ...)` call with `lock acquisition failed because the operation would block`, even though no other thread in the binary touches that private fixture directory. The cause is an operating-system property of `flock`, not a defect in the assertion: `flock` ownership belongs to an open file description, and `posix_spawn` copies the parent's whole descriptor table into the child before image activation closes `FD_CLOEXEC` descriptors, so a concurrently spawning thread transiently holds a duplicate of a lock file's open file description. When the parent releases its own descriptor inside that window, the lock stays held and the next `try_lock` returns `EWOULDBLOCK`. `cargo test -p kuru --lib` runs the lock-taking tests in parallel with the binary's memory-store opens and `/bin/sh` spawns, which supplies exactly that concurrency.

## What Changes

Add a crate-internal, test-only gate in `apps/kuru-tui` that serialises advisory-lock acquisition against in-binary process spawning: tests that take a store or trust lock hold it exclusively, tests that spawn a child process hold it shared, so spawning tests still run concurrently with one another. No production code, no assertion, no timeout and no retry changes; the tested fail-closed behaviour is unchanged and the flaky failure mode is removed at its cause rather than tolerated.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

New `#[cfg(test)]` module `apps/kuru-tui/src/spawn_gate.rs` (declared in `lib.rs`) with its regression tests; the exclusive guard is taken by the eight `#[test]` functions in `permission_store.rs`, the five in `trust.rs` and the permission test in `cli.rs`; the shared guard is taken across every child-process creation in the binary's tests — `MemoryStore::temporary`/`MemoryStore::open` in `ui/runtime_tests.rs` and `memory_notice.rs`, and both `/bin/sh` spawns in `authentication.rs`. No public API, configuration, capability spec or shipped behaviour changes.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
