## Context

`File::try_lock`/`File::lock` acquire an advisory lock with
`flock(LOCK_EX | LOCK_NB)` on Unix. `flock` ownership belongs to the *open
file description*, not to a descriptor or a process, so any duplicate of that
description anywhere keeps the lock held until the last duplicate closes.
`posix_spawn` (Rust's `Command::spawn`/`tokio::process::Command::spawn`) on
XNU copies the whole parent descriptor table into the new process and only
closes `FD_CLOEXEC` descriptors at image activation, strictly after the copy,
so `O_CLOEXEC` does not close the window. A thread spawning a child while
another thread releases (or is about to release) a lock file therefore leaves
the advisory lock transiently held by the child's duplicate, and the next
`try_lock` on a freshly opened descriptor returns `EWOULDBLOCK`. This is the
exact mechanism already diagnosed and fixed for `apps/kuru-tui` in the
archived `openspec/changes/archive/2026-09-18-serialize-test-lock-and-spawn`,
whose `apps/kuru-tui/src/spawn_gate.rs` this change mirrors.

`packages/kuru-memory`'s own `--lib` test binary runs the same shape:
`server_tests.rs`'s `supervisor_rejects_bad_configuration_and_parent_eof_without_spawning`
opens and locks `lifecycle.lock` itself, drops it, and expects a third
`supervise(read_only: true, ...)` call to observe "not been initialized";
meanwhile `provision/native_tests.rs`, `store/recovery_tests.rs`,
`store/migration_lifecycle_tests.rs` and `store/operational_gc_tests.rs` spawn
real Dolt supervisors and engines concurrently in the same binary. Walking
`supervise`/`supervise_with_port_hook` (`server.rs`) confirms the flaky
assertion is reached only when the lock still *appears* held on the third
call, at which point the code falls into the same "parent closed" bail as the
deliberately-contended second call.

## Goals / Non-Goals

**Goals:** Remove the flaky failure at its cause inside the `kuru-memory`
library test binary, and give the crate's own real-Dolt behavioral fixtures
(the four files above) broad coverage without hand-editing each of their
individual tests.

**Non-Goals:** Do not retry, widen a timeout, weaken an assertion or ignore a
test. Do not change `server.rs`, `engine.rs` or `provision.rs`'s own
locking/spawning logic — production behavior is unchanged.

## Decisions

- Ownership of the fix is test-only, mirroring `apps/kuru-tui/src/spawn_gate.rs`
  exactly in mechanism (a single `tokio::sync::RwLock<()>` in a `#[cfg(test)]`
  module; lock-taking code holds the write guard, spawning code holds the
  read guard). Gating a production call site (e.g. `server.rs`'s owned
  supervisor spawn, or `provision.rs`'s `cache_lock`) was considered and
  rejected: several fixtures deliberately hold a raw lock *through* a real
  spawn by design (`store/migration_lifecycle_tests.rs`'s
  `interrupted_migration_close_handoff_retains_guard_until_supervisor_quiesces`
  passes its own held guard into `Server::open_with_guard`, per AGENTS.md's
  "hold the stable lifecycle lock through migration/recovery directory
  moves"); a production-side gate taken inside that same call would self-
  deadlock against a test already holding the exclusive guard on the same
  task. Kept entirely test-side, the gate can never nest with itself.
- Both a lock-taking test and a spawning test can safely hold their guard for
  their *whole* body when neither also does the other kind of gated operation
  in between (the common case, matching `apps/kuru-tui`'s own usage); a test
  that legitimately needs both (e.g. `owner_drop_transfers_installed_reap_guard_until_real_child_reaps`,
  which locks a file, spawns a real child, then drops the lock) takes only
  `locking`/`locking_async` around the operations it performs directly — a
  spawn it performs *while already holding the exclusive guard* needs no
  separate `spawning` call, since the exclusive guard already excludes every
  concurrent spawn; a test that must release a raw lock across an `await`
  boundary (`cleanup_observation_error_keeps_actual_lifecycle_lease_until_child_exit`'s
  `drop(lease)` inside a spawned task) brackets that drop with its own guard
  acquisition, separately from the earlier one.
- `store/recovery_tests.rs`, `store/migration_lifecycle_tests.rs` and
  `store/operational_gc_tests.rs` call `MemoryStore::open` directly at ~34
  individual sites; `provision/native_tests.rs` calls `cache_lock`/
  `verify_version` at ~20 sites. Editing each individually was rejected as
  impractical and error-prone; instead each file gets (or, for the three
  `MemoryStore::open` files, shares) one new `#[cfg(test)]` choke-point
  helper (`crate::test_support::spawn_gated_open`,
  `provision/native_tests.rs`'s `gated_cache_lock`/`gated_verify_version`)
  that takes the gate once, and every call site is mechanically rewritten to
  call the gated helper instead of the raw function. This is the same
  "single choke point" shape as `apps/kuru-tui`'s own
  `MemoryStore::temporary`/`MemoryStore::open` gating, adapted to live inside
  `kuru-memory` itself (where the crate's own tests cannot reach a *caller-
  side* wrapper the way an external consumer crate can) rather than at the
  true OS-level spawn/lock call (which is production code, ruled out above).
- The regression test is `spawn_gate.rs`'s own private-`Gate`-instance test,
  copied verbatim in mechanism from `apps/kuru-tui/src/spawn_gate.rs`'s
  `an_in_flight_spawn_excludes_lock_acquisition_until_it_finishes`: a thread
  blocked on the exclusive side publishes its acquisition through a channel,
  and the shared holder observes nothing published before it releases, then
  observes acquisition afterwards — no wall-clock wait, so the test cannot
  itself flake.
- The gate is a `tokio::sync::RwLock`, not `std::sync::RwLock`, because
  spawning sites hold the shared guard across `.await`. Synchronous tests
  take it with `blocking_write`/`blocking_read` (the latter is a
  `kuru-memory`-specific addition, `spawning_blocking`, needed because two of
  `server_tests.rs`'s spawning tests are plain `#[test]` functions with no
  Tokio runtime — `apps/kuru-tui`'s spawning tests are all `#[tokio::test]`
  and never needed this variant).

## Risks / Trade-offs

Coverage is exhaustive for every test/helper that itself opens and locks a
file, or itself spawns/drives `MemoryStore::open`/`Server::open`/
`crate::engine::spawn`. It is *not* exhaustive for the moment a lock **owned
by production code** is eventually released after being handed across a
gated boundary — `interrupted_migration_close_handoff_retains_guard_until_supervisor_quiesces`'s
guard is gated only at its acquisition (the test hands it to
`Server::open_with_guard`, and its eventual release happens inside `Server`'s
own reap machinery, which this change does not instrument, consistent with
"no production code changes"). This mirrors the same accepted residual risk
`apps/kuru-tui`'s design records for `GrantStore`: Kuru fails closed on a
transiently-contended lock, which is the correct posture, not a correctness
regression — at worst a bounded flake in the same class as the one this
change fixes, not a wrong result.

Lock tests can wait behind an in-flight memory open; those waits are bounded
by the opens' own existing behavior. A future test that spawns a child
process, or opens/locks a file, without taking the shared or exclusive guard
reopens the window for that specific operation; the module documentation
states this explicitly at the definition.

## Operational surface

None. The gate and its two `#[cfg(test)]` choke-point helpers compile only
under `cfg(test)` and never reach a shipped binary, installation, packaging
or documentation surface.
