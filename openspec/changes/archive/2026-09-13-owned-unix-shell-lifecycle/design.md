## Context

The Unix built-in shell currently keeps a numeric process-group guard separate
from its Tokio child. Natural completion reaps the root before the guard's Drop
signals the group, while timeout, capture failure, and caller cancellation drop
both without awaiting reap or absence. A reused numeric group could therefore
be signalled after the identity anchor is gone, and `ToolHost::shutdown` does
not know about live shell calls.

The delivery package has useful `waitid(WNOWAIT)` and bounded-observation
evidence, but its runner is package policy rather than a connector dependency.
Pinned rustix 1.1.4 already exposes the safe process calls needed by platform;
pinned Tokio 1.53.1 converts standard child pipes to async readers.

## Goals / Non-Goals

**Goals:**

- Keep root identity valid through the last destructive group/root signal and
  make post-reap signalling structurally unavailable.
- Preserve exact status/output on natural completion and the primary error on
  forced cleanup.
- Keep shell ownership alive after caller-future or parent-runtime loss, and
  integrate bounded cancellation with `ToolHost::shutdown`.
- Return to callers within the operation deadline plus one cleanup allowance,
  while retaining ownership when cleanup cannot yet be confirmed.

**Non-Goals:**

- OS sandboxing, containment of processes that leave the group, or an atomic
  Unix owner-death primitive.
- Changes to Windows Jobs, stdio MCP, delivery commands, runtime journaling,
  memory ownership, permission language, or shell concurrency policy.

## Decisions

### Platform owns the standard child and signal state

Add a small `cfg(unix)` platform module. `OwnedProcessGroup::spawn` consumes a
fully configured `std::process::Command`, adds `process_group(0)`, and retains
the `std::process::Child`; it exposes only owned stdout/stderr pipe extraction.
The connector cannot wait, reap, or obtain the PID independently.

The owner has internal `Anchored`, `PostSignal`, `Reaped`, and `Disarmed`
states. `root_state` uses `waitid(EXITED | NOHANG | NOWAIT)`. Each call retries
`EINTR` for at most eight total syscall attempts without wall-clock policy.
Exhausted interruption returns a distinct result and leaves `Anchored`
authority intact for a later connector poll. `ECHILD` and other
ownership-invalidating errors permanently disarm the object and never regain
authority.

`terminate_before_reap` repeats that ownership observation itself. Interruption
sends nothing and preserves authority. Once actual destructive attempts begin,
it consumes authority and signals the original group first and root second;
separate root signalling handles a root that moved from the original group.
`reap_if_exited` rejects `Anchored`, polls without blocking, and calls standard
`Child::wait` only after exit was observed. It caches the exact status. After
reap, only signal-zero presence observation remains; `ESRCH` alone means absent
and `EPERM` remains an unconfirmed bounded observation.

Dropping an unexpectedly anchored object first repeats the ownership check and
may request group/root termination only while still anchored. It cannot claim
reap. Post-signal, reaped, and disarmed Drop never signal. This is a panic
backstop; ordinary ownership remains with the connector worker.

### Connectors owns capture, deadlines, and an independent worker

Each accepted built-in Unix shell call registers a private owner, then starts a
named OS thread with its own current-thread Tokio runtime. The worker owns the
platform object, converted async stdout/stderr, retained workspace
`Arc<Directory>`, command, environment, and result state. It revalidates the
root immediately before platform spawn. `ChildStdout::from_std` and
`ChildStderr::from_std` keep async pipe readiness out of platform.

Capture incrementally drains both streams to the existing 2 MiB-per-stream
limit and polls non-reaping root state fairly. Natural completion requires both
EOFs and root exit. It then signals group/root before reap, caches the original
status, and waits for group absence before emitting the unchanged successful
JSON. Timeout, overflow, read failure, caller closure, shutdown, or a caught
post-spawn panic records its primary failure and enters the same cleanup.

The operation deadline is calculated before registry insertion. The caller
also has an absolute fallback at operation deadline plus five seconds; this is
one cleanup allowance, not a second sequential window. On fallback it requests
cancellation, closes the result receiver, and reports fixed unconfirmed
cleanup. A controlled worker delayed before start remains registered, later
observes cancellation/deadline, launches nothing, and removes itself.

Once cleanup begins, connectors owns a fresh five-second confirmation deadline.
It repeatedly invokes the platform's nonblocking mechanics; platform receives
no expired deadline. If confirmation misses the caller bound, the worker sends
the bounded result but keeps the platform child, pipes, and workspace guard and
continues cleanup. If bounded `EINTR` left the child `Anchored`, a later valid
ownership observation may still begin its one destructive group/root transition.
Once that transition has started, delayed confirmation is strictly
non-signalling: it only reaps and observes absence. Later confirmation removes
the registry entry instead of being disabled forever by the earlier deadline.

### Registration is visible before thread start

Create result, cancellation, and completion controls first. Under a short
standard mutex, reject a closing registry or insert one complete `Starting`
entry, then release the mutex before thread creation. Shutdown cancels starting
and active entries. A thread that starts after shutdown checks cancellation,
caller closure, and the accepted deadline before runtime construction and again
before spawn, so it cannot launch late.

Synchronous thread-start failure removes only its reservation and launches
nothing. Runtime failure and pre-spawn panic do the same through a weak
self-removal guard. Post-spawn panic is caught while the owned session remains
outside the caught future and uses ordinary cleanup. Confirmed workers remove
their own entry through a weak registry reference, so a long-lived harness does
not accumulate completed calls. No hard concurrency cap is added; the registry
contains only starting, active, or deliberately retained unconfirmed owners.

`ToolHost::shutdown` first closes registration, cancels all owners, and waits
one five-second window while also completing existing MCP shutdown. It combines
fixed errors so neither cleanup is skipped. Synchronous Drop requests
cancellation but does not claim it waited. The current Harness clears actors
then awaits tools; the shell worker retains no `MemoryStore` or Dolt writer
lease, and no runtime API change is required.

## Integration contract

The initial `cfg(unix)` platform contract is a non-`Clone`
`OwnedProcessGroup` with `spawn(std::process::Command)`, one-shot
`take_stdout`/`take_stderr`, and mutable `root_state`,
`terminate_before_reap`, `reap_if_exited`, and `presence_after_reap` methods.
No method exposes the child or a PID. Typed outcomes distinguish running,
exited, interrupted, ownership lost, unexpected observation, sent, already
absent, permission denied, not attempted, cached exact `ExitStatus`, and invalid
phase. `terminate_before_reap` returns one report containing the ordered group
and root signal outcomes; once it begins, another call cannot signal. Only test
builds inject syscall or worker-start observations.

Expected platform files are `packages/kuru-platform/src/lib.rs`, a small Unix
process-group module, its crate/module description, and platform tests. Expected
connector files are `packages/kuru-connectors/src/tools.rs`, a private Unix
shell-owner module, focused unit/native fixtures, `AGENTS.md`,
`docs/protocols.md`, and `apps/kuru-docs/reference/tools.md`. The additional
`apps/kuru-tui/tests/unix_shell_turn.rs` fixture runs a real conversational CLI
turn against an isolated fake Responses server. Its outer CLI process reuses
delivery's bounded runner in an independent runtime scope retaining its sandbox
through completion. The TUI manifest enables the existing `kuru-delivery`
`tooling` feature as a dev-dependency for this helper, without a new version or
lockfile pin. Production connectors do not consume delivery's runner.

The CLI fixture proves the exact typed shell receipt in the provider continuation,
the final JSON answer, and normal command exit. Existing native terminal fixtures
separately verify raw terminal restoration; `run --json` does not enter the TUI
and must not be reported as proving raw terminal restoration.

## Operational surface

This changes the local built-in shell reached through Kuru's existing tool and
terminal flows. It adds no bind address, listener, container, remote runner,
required secret, or provider credential. The command still runs as the current
host user with the existing finite environment projection and remains explicit
process authority rather than a sandbox.

Existing command timeout and 2 MiB-per-stream output limits remain. There is no
new product concurrency setting or hard registry capacity; one private OS thread
exists for each accepted shell call that is starting, active, or awaiting
confirmed cleanup, and completed entries are reclaimed during a long-lived host.
No bundled binary, architecture, runtime dependency, or version changes. The new owner
is native to macOS and Linux; the existing Windows Job path remains unchanged.

## Risks / Trade-offs

- **One OS thread per concurrent shell costs more than a Tokio task.** → It is
  limited to accepted active/unconfirmed shell calls and is the smallest owner
  that survives parent-runtime destruction; completed entries self-remove.
- **A persistent permission result or failed signal may never prove cleanup.**
  → Return a bounded unconfirmed error and retain the independent owner and
  workspace capability; never turn `EPERM` into success.
- **Standard `Child::wait` is synchronous.** → Call it only after non-reaping
  waitid reports exit, making it an immediate reap rather than blocking runtime
  work.
- **The process-group root or descendants may deliberately change groups.** →
  Signal original group then root while root identity remains anchored; make no
  claim over escaped descendants.
- **Platform Drop cannot await.** → Catch normal panic/error paths in the
  independent worker and treat Drop signalling only as an unconfirmed emergency
  request.
