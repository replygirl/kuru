## Context

`trust::lock` opens the lock file through `Directory::lock_file` (`packages/kuru-platform/src/fs/unix.rs`, `O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK`) and calls `File::try_lock`, which on macOS is `flock(LOCK_EX | LOCK_NB)`. `flock` ownership is a property of the open file description, so any duplicate of that descriptor in any process keeps the lock held until the last duplicate closes. `posix_spawn` on XNU copies the parent's descriptor table into the new process and only closes `FD_CLOEXEC` descriptors at image activation, which is strictly after the copy; `O_CLOEXEC` therefore does not close the window. If the parent drops its own descriptor inside that window, the advisory lock remains held by the child's duplicate and the next `try_lock` on a freshly opened descriptor returns `EWOULDBLOCK`.

A standalone C reproduction on this host (Darwin 27.0.0, aarch64) isolates the mechanism: one thread repeatedly opens, `flock`s and closes a file no other thread touches, while N threads loop `posix_spawn("/bin/sh", "-c", "exit 0")` + `waitpid`. With six spawner threads, 756,226 lock attempts produced 7,636 `EWOULDBLOCK` (~1.0%); with zero spawner threads, 505,735 attempts produced none.

## Goals / Non-Goals

**Goals:** Remove the flaky failure at its cause inside the `kuru` library test binary, and prove the serialisation ordering deterministically.

**Non-Goals:** Do not retry, widen a timeout, weaken an assertion or ignore a test. Do not change `trust::lock`, `GrantStore` or any production spawn path.

## Decisions

- Ownership of the fix is test-only, because the failure is a property of running lock acquisition and process spawning concurrently inside one binary. A blind retry inside `trust::lock` was rejected: it would also paper over a genuinely contended lock, which is exactly what `held_store_lock_and_in_workspace_storage_fail_closed` asserts must fail closed.
- The gate is a single `RwLock<()>` in a `#[cfg(test)]` module at the crate root. Lock-taking tests hold the write guard, spawning tests hold the read guard, so spawning tests still run concurrently with one another and only serialise against lock acquisition. The gate is `tokio::sync::RwLock` (see the next decision), which does not poison: a panicking holder simply drops its guard, releasing the lock normally, so one failing test does not cascade into unrelated failures.
- Both sides are gated exhaustively, not only at the two sites named in the original diagnosis. Every advisory-lock test in the binary takes the exclusive guard (`permission_store.rs`, `trust.rs`, `cli.rs`) and every child-process creation takes the shared guard (`MemoryStore::temporary` as well as `MemoryStore::open`, plus the `/bin/sh` spawns). A partial gate leaves the window open: an ungated spawn still duplicates the descriptor table, and a soak observed exactly that before `MemoryStore::temporary` and the `cli.rs` approval test were included.
- The gate is a `tokio::sync::RwLock`, not `std::sync::RwLock`, because the spawn sites hold the shared guard across `await`. Synchronous lock tests take the guard with `blocking_write`; asynchronous ones take it with `locking_async`.
- The regression test proves the ordering without wall-clock waits: a thread blocked on the exclusive side publishes its acquisition through a channel, and the shared holder observes that nothing has been published before it releases, then observes acquisition afterwards. The observation is injected rather than timed, so the test cannot itself flake.

## Risks / Trade-offs

Lock tests can wait behind an in-flight memory open; those waits are bounded by the opens' own existing behaviour, and the observed cost is none — the library suite's wall-clock time is unchanged. A future test that spawns a child process without taking the shared guard would reopen the window, which the module documentation states explicitly at the definition.

The same operating-system hazard remains reachable in the product: a tool spawn concurrent with a grant read or write can make `GrantStore::load/add/revoke` report the transient block. Kuru fails closed there — no grant is written and no authority is granted or silently reused — which is the correct posture, and it is the same class of hazard AGENTS.md already records for Windows handle inheritance. A process-wide spawn gate in `kuru_platform` would narrow it for Kuru-owned spawns only, cannot cover third-party spawns, and is a production behaviour change on a hot path; it is recorded as a known limitation instead of being taken here.

## Operational surface

None. The gate compiles only under `cfg(test)` and never reaches a shipped binary, installation, packaging or documentation surface.
