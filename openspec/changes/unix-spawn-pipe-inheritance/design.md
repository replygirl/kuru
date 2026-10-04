# Design

## Context

### Root cause (std source, rust 1.98.1, commit 48a229cea)

Paths below are under `library/std/src/sys/` of the pinned toolchain's source
(the task's guessed `sys/pal/unix/pipe.rs` no longer exists).

- `pipe/unix.rs:14-30`: Android, the BSDs, illumos, Hurd, Linux, Cygwin and
  Redox create pipes with `pipe2(fds, O_CLOEXEC)`, atomic.
- `pipe/unix.rs:32-41`: every other Unix, including all Apple targets, calls
  `pipe()` and then `set_cloexec()` on each end; `fd/unix.rs:552-574` makes that
  a separate `ioctl(fd, FIOCLEX)` on Apple. Between `pipe()` and the second
  ioctl both ends are inheritable.
- `process/unix/common.rs:423-427`: `Stdio::MakePipe` (what `Stdio::piped()`
  becomes) calls that `pipe()` in `setup_io`.
- `process/unix/unix.rs:71`: `setup_io` runs before either spawn path;
  `:93` and `:770` take `env_read_lock()`, a shared read lock held by every
  concurrent spawn and taken after the pipes exist. No other lock in std
  orders pipe creation against another thread's spawn, and Apple's `pipe()` is
  the plain syscall.
- posix_spawn path (`:456-837`): `process_group(0)` becomes
  `POSIX_SPAWN_SETPGROUP` (`:729-731`), so it does not force fork. posix_spawn
  copies the descriptor table at the call and closes close-on-exec descriptors
  only at image activation (as `kuru-memory/src/spawn_gate.rs` already records
  for `flock`). std never sets Apple's `POSIX_SPAWN_CLOEXEC_DEFAULT`
  (`:698-767`).
- Fork path: taken when `pre_exec` closures, uid/gid, groups or chroot are set,
  when the environment was cleared or `PATH` changed and the program has no
  `/` (`:467-475`; `env_clear` counts, `process/env.rs:76`), or on Apple when a
  working directory is set and the program is relative (`:650-658`). Hooks and
  the built-in shell use `env_clear` with bare programs such as `sh`, so they
  often take it. That path also creates its exec-error pipe with the same
  non-atomic `pipe()` (`:81`) and blocks reading it to EOF (`:138-161`); a
  concurrently spawned child that inherits its write end stalls the spawn
  call itself until that child exits.

So the window exists on macOS for both spawn paths and in both directions, and
it is consistent with the measured lsof evidence (an extra, high-numbered pipe
fd in the already-started hook whose peer is the other hook's stdin).

### Owned spawn inventory (product sources)

Owned (through `kuru-platform`; receives the guarantee):
- `kuru-connectors/src/hooks.rs:1018` lifecycle hooks (pipe ×3).
- `kuru-connectors/src/unix_shell.rs:846` built-in shell (null/pipe/pipe).
- `kuru-connectors/src/rpc.rs:405` MCP stdio servers (pipe ×3).
- `kuru-delivery/src/coverage.rs:1848` coverage runner group
  (inherit/pipe/inherit; delivery tooling, not the application).
- `kuru-platform/src/unix/snapshot.rs:316` bounded `ps` listing (null/pipe/pipe),
  drained to EOF.

Legacy (unchanged, outside the guarantee): `kuru-memory` supervisor
(`server.rs:649-675`, tokio, stdin+stdout piped, `process_group(0)`), Dolt
engine (`engine.rs:21-45`), project memory service (`service.rs:1355-1383`),
version probes (`provision.rs:1955`); `kuru-delivery` command runner
(`command.rs:381`), open-time launcher (`open_time/launch.rs:114-178`),
coverage orchestration (`coverage/orchestrate.rs:438`), updater/census
commands; `kuru-tui` git listing (`cli.rs:637`), updater (`cli.rs:2022`) and
browser opener (`authentication.rs:79-130`). Test-only spawns are not
inventoried. No product code uses `pre_exec`. Windows spawns go through
`NativeSpawnSpec` and its handle list and are untouched.

## Goals / Non-Goals

**Goals:**
- No owned Unix child inherits another owned spawn's pipe end, on any Unix.
- A regression test that fails deterministically without the lock on macOS.
- The guarantee and its limit documented beside the Windows note.

**Non-Goals:**
- Isolation from legacy spawns or from other non-atomic descriptor creation
  (sockets on Apple are also `socket()` + FIOCLEX). Migrating the memory
  supervisor is the highest-value follow-on and a separate change.
- Any timeout, retry, bound or `HookBudget` concurrency change.
- Replacing std's spawn (own `posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT`).

## Decisions

1. **One process-wide platform spawn lock.** A private
   `static SPAWN: Mutex<()>` in `kuru_platform::unix`, taken with
   `lock().unwrap_or_else(PoisonError::into_inner)` (it guards no data), held
   from before the first pipe is created until `Command::spawn` returns, and
   released before any caller code runs. Acquisition is `try_lock` then
   `lock`, so a test-only seam can report "blocked" between the two. Taken by
   `OwnedProcessGroup::spawn` and `snapshot::list_within` only.
   Alternative rejected: wrapping only `command.spawn()` (std creates the
   pipes). It closes the same owned-vs-owned window, but the window then lives
   inside std where no seam can pause it, so the regression could only be
   probabilistic (measured ~1% of iterations at load 50, unknown on an idle
   runner).

2. **The platform creates the pipes.** `OwnedProcessGroup::spawn(command,
   stdio)` takes a `StdioPlan { stdin, stdout, stderr }` of
   `StdioSlot::{Pipe, Null, Inherit}` and sets all three on the `Command`,
   replacing anything the caller set. Pipes come from
   `rustix::pipe::pipe_with(PipeFlags::CLOEXEC)` where available and
   otherwise (Apple) `rustix::pipe::pipe()` followed by close-on-exec on both
   ends (`ioctl_fioclex`, as std does), under the lock. The child end is
   passed as `Stdio::from(OwnedFd)`; std dup2s an end above 2 onto the
   standard descriptor as `ChildStdio::Explicit` (`common.rs:410-417`,
   `unix.rs:704-724`), and an end that lands on 0-2 (a standard descriptor
   closed in the parent) takes std's `duplicate()` path, which is
   `F_DUPFD_CLOEXEC` and atomic (INFERRED: `FileDesc::duplicate` is outside
   the sparse clone's cited lines), so neither creates an inheritable copy.
   The parent end is kept as `OwnedFd` and returned by the unchanged `take_*`
   methods via `ChildStdin/ChildStdout/ChildStderr::from(OwnedFd)` (INFERRED
   stable since 1.74: `os/fd/owned.rs` and `process.rs` are not in the sparse
   clone; the build proves the conversions exist on the pinned toolchain).
   `Command` is consumed and dropped explicitly (`drop(command)`) inside
   `spawn` before the lock guard is released, closing the parent's copies of
   the child ends; otherwise the parent would hold its own child's stdout
   write end and never read EOF. A parameter outlives a local guard, so the
   explicit drop is what orders it.
   `Stdio::null()` opens `/dev/null` with `O_CLOEXEC` (atomic) and `Inherit`
   creates nothing, so std makes no inheritable stdio descriptor. The
   exec-error pipe of std's fork path is still std's, but it is created inside
   `command.spawn()` under the lock, so owned spawns cannot inherit it either.
   Alternative rejected: reading stdio back from `Command`: there is no getter,
   and the native-platform spec forbids lossy `Command` introspection.

3. **Snapshot spawn under the same lock.** `ps` keeps std pipes but its
   `.spawn()` is wrapped by the lock (its own window is then inside the lock),
   since an owned child inheriting `ps`'s stdout write end would stall the
   diagnostic drain until the bounded snapshot timeout.

4. **Forced-race regression (unit test in `src/unix.rs`, every Unix).**
   Two thread-local `cfg(test)` seams, each taken and fired at most once:
   `window`, fired only while creating the STDIN slot's pipe (between `pipe()`
   and close-on-exec on Apple, after `pipe_with(CLOEXEC)` elsewhere); a plan
   whose stdin is not a pipe never fires it, so no later pipe can be paused
   after stdin is already close-on-exec. `blocked` is fired when `try_lock`
   finds the lock held, before `lock`. Thread A spawns `cat` (stdin pipe, stdout pipe, stderr
   null); its `window` seam fires once, signals B, then waits on a channel for
   B's first event, either `Blocked` (from B's seam) or `Spawned` (sent by B's
   test code after its spawn returns), bounded by the five-second bound the
   sibling owner unit tests already use (`unix.rs:1015`, `:1027`; no new
   constant). A never waits for `Spawned` alone, so the fixed build cannot
   deadlock on itself. Thread B spawns `cat` with stdin pipe once A is in the
   window. With the lock, B reports `Blocked`, A finishes, and B spawns after
   A's pipes are close-on-exec. Without the lock, B's spawn copies A's pipe
   ends and reports `Spawned`; on Apple those ends are not yet close-on-exec
   and survive B's exec. The test then drops A's stdin and requires A's stdout
   to reach EOF (A's `cat` exited) within the bound while it still holds B's
   stdin open, and asserts B's first event was `Blocked`. The EOF wait is a
   reader thread on A's stdout plus `recv_timeout` against the bound; the test
   thread never blocks on a read, and on any failure it still drops B's stdin
   and cleans both groups before panicking. Only afterward does
   it drop B's stdin and clean both groups up through the existing owner
   transition (reap after stdout EOF, bounded by the same deadline, yielding
   rather than sleeping). The leak self-resolves when B's child exits, so that
   order is what makes the unfixed build fail. No sleep. The test runs on
   every Unix, proving the lock property (`Blocked`) on Linux too and keeping
   the seam lines covered there; only Apple can fail the EOF assertion without
   the lock, because Linux pipes are atomic. A unit test on every Unix also
   asserts the created pipe ends carry close-on-exec.

5. **Natural concurrency sentinel (integration test).**
   `tests/unix_process_group.rs`: 50 iterations, two threads released by a
   `Barrier`, each spawning `cat` with piped stdin; A's stdin is dropped and
   A's stdout must reach EOF within `LIMIT` while B's stdin is still held;
   then B. This must pass with the fix; it is not claimed to fail reliably
   without it (measured ~1% per iteration, load-dependent).

6. **Docs.** `unix.rs` module doc mirrors `windows/process.rs:1-6`; AGENTS.md
   gains one sentence beside "Unix process-group operations retain..."; and
   `docs/development.md` (the `kuru-platform` paragraph) states the guarantee
   and limit. Proposed text: "Owned Unix spawns create their stdio pipes
   close-on-exec and start the child under one platform spawn lock, so a
   concurrent owned child cannot inherit another's pipe ends (std's macOS pipes
   set close-on-exec in a second step). Unrelated legacy spawns and other
   non-atomic descriptor creation can still inherit, or leak into owned
   children; concurrent callers requiring isolation must use the platform
   consistently. On std's fork path, a legacy spawn that inherits an owned
   spawn's exec-error pipe stalls that spawn until the legacy child exits, and
   every later owned spawn, including the cleanup snapshot, waits behind it;
   the remedy is moving those legacy spawns behind the platform, not a
   timeout."

## Risks / Trade-offs

- [Deadlock] The lock is a leaf: no other lock is taken under it, no caller
  code runs under it (`pre_exec` closures run only in the child), and it is not
  ordered with the memory crate's test-only tokio gates. A forked child gets a
  locked copy but execs or `_exit`s without touching it. Poison is ignored.
- [Stall amplification, acknowledged severity change] On std's fork path the
  spawn blocks reading its exec-error pipe. If a legacy spawn (memory
  supervisor `server.rs:675`, Dolt engine `engine.rs:45`) inherits that write
  end in its own window, that owned spawn stalls until the legacy child exits.
  That per-spawn stall is pre-existing; with the lock it becomes process-wide
  for owned spawns: the stalled spawn holds the lock and every later owned
  spawn (tools, MCP, hooks) and the cleanup `ps` snapshot waits behind it, for
  a Dolt-lifetime child unboundedly. The window requires a legacy spawn at
  that instant; legacy spawns are startup/login/update paths plus memory
  service restarts. Mitigation is migrating the memory supervisor and engine
  spawns behind the platform (follow-on), not a timeout. Acknowledged through
  the lead's instruction to implement with fable's required changes.
- [Async runtime] `OwnedProcessGroup::spawn` is synchronous and is called from
  tokio tasks (`unix_shell`, `rpc`). A worker can block for the owned spawns
  queued ahead of it: syscall work of about a millisecond each on macOS, no
  `.await` under the lock.
- [Performance] Owned spawns are serialised; they are rare (hooks per turn,
  tool calls, MCP startups). Locked and unlocked 2000-iteration experiments
  took 19.4 s and 24.3 s (dominated by waits; no measurable cost).
- [API churn] Four callers and platform tests change to declare stdio; a
  caller's own `.stdin()/.stdout()/.stderr()` is overwritten, documented on
  the method.
- [posix_spawn interaction] Unchanged: std still chooses posix_spawn or fork;
  the lock covers both, and child ends reach 0-2 via `adddup2`/`dup2`, which
  clears close-on-exec on the target descriptor only.
