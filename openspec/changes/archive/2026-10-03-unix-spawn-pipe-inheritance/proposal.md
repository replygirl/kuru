# Proposal

## Why

Two lifecycle hooks launched concurrently on macOS deadlock about once in two
hundred runs, each hang lasting exactly until the first hook's own deadline
(`hooks::tests::shared_budget_holds_leases_through_owned_reap_and_caps_invocations`,
macos-latest coverage, run 37156795961; reproduced locally 13 times in about
2000 runs). `lsof` during five hangs showed the hook that had started holding
the write end of the other hook's stdin pipe after the parent had dropped its
own copy, so the second hook's `request=$(cat)` never saw end of file.

The cause is in the owned spawn path. `OwnedProcessGroup::spawn` hands a
`Command` with `Stdio::piped()` to std, and std's Unix pipe creation on Apple
targets is `pipe()` followed by a separate `ioctl(FIOCLEX)` on each end
(rust 1.98.1 `library/std/src/sys/pipe/unix.rs:32-41`,
`library/std/src/sys/fd/unix.rs:569-574`). Another thread's `posix_spawn` or
`fork` in that window copies both ends without close-on-exec, and they survive
the other child's `exec`. Nothing in std serialises pipe creation against a
concurrent spawn: the environment lock is a shared read lock taken after the
pipes exist. Linux uses `pipe2(O_CLOEXEC)` and has no window for these pipes.
Every concurrent owned launch (hooks, the built-in shell, MCP stdio servers) is
exposed in both directions, and on std's fork path the same window can stall
the spawn call itself on its exec-error pipe.

## What Changes

- `kuru_platform::unix::OwnedProcessGroup::spawn` creates the child's stdio
  pipes itself, close-on-exec atomically where the OS allows (`pipe2` on
  Linux) and otherwise before any other platform spawn can copy the
  descriptor table, and starts the child while holding one process-wide
  platform spawn lock. Child ends are passed to std as explicit descriptors, so
  std creates no stdio pipe for an owned spawn.
- Callers declare stdio to the owner (pipe, null or inherit per stream)
  instead of configuring `Stdio` on the `Command`; the platform cannot read a
  `Command`'s stdio back and must not reconstruct it. The `take_stdin`,
  `take_stdout` and `take_stderr` contract is unchanged.
- The platform's bounded `ps` snapshot spawn takes the same lock.
- The lock is private to `kuru-platform` and taken only by the platform's own
  spawns; legacy spawns elsewhere are unchanged and documented as outside the
  guarantee, exactly as the Windows handle-list note does.
- A deterministic regression test forces the window through a test-only seam
  between pipe creation and close-on-exec; a concurrent tight-loop test covers
  the natural race.

## Capabilities

### New Capabilities

### Modified Capabilities

- `native-platform`: the owned Unix process group now owns stdio pipe creation
  and states the concurrent-inheritance guarantee and its limit.

## Impact

- `packages/kuru-platform/src/unix.rs`, `src/unix/snapshot.rs`,
  `packages/kuru-platform/Cargo.toml` (rustix `pipe` feature on the existing
  pinned crate; no new dependency, no lockfile version change).
- Callers: `packages/kuru-connectors/src/hooks.rs`, `src/unix_shell.rs`,
  `src/rpc.rs`; `packages/kuru-delivery/src/coverage.rs`; platform tests.
- Docs: `AGENTS.md`, `docs/development.md`, the platform module docs.
- No timeout, retry or `HookBudget` concurrency change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
