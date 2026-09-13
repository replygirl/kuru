## Context

`apps/kuru-tui/src/ui.rs` currently drains broadcast activity and typed
completions, renders, then calls blocking `crossterm::event::poll` for 25 ms while
busy or 100 ms while idle before `event::read`. The Tokio worker cannot service a
completion or activity wake during that call. The pre-completion activity drain
also runs until `try_recv` becomes empty, so a continuous producer can postpone
the typed result that authoritatively finishes the operation.

The existing terminal owner already restores raw mode, alternate screen,
bracketed paste, focus reporting and Windows console modes around `run_loop`, but
an early draw/read error drops the
`JoinHandle` and detaches a live TUI dispatch task. The scheduler fix must close
that task before returning to terminal restoration while leaving Harness and
runtime cancellation policy unchanged.

## Goals / Non-Goals

**Goals:**

- Wake immediately for terminal input, typed completion, activity and animation.
- Preserve generation fencing, typed result authority, completion locking,
  completion-before-refresh, failed refresh plus settle, cancellation ordering,
  command output, motion cadence and reduced motion.
- Give continuously ready sources a deterministic service bound.
- Restore terminal state after input EOF/error and draw/backend failure without
  detaching TUI-owned work.

**Non-Goals:**

- No public event abstraction, `TurnOutput`/JSON change, streaming response
  protocol, provider/runtime ownership change or new shutdown policy.
- No new event buffer, crossterm/futures version, terminal design, animation
  cadence, command behavior or persistence behavior.

## Decisions

### One private async terminal reader

Enable crossterm 0.29.0's `event-stream` and `use-dev-tty` features on the TUI's
inherited dependency and add the workspace-pinned futures 0.3.34 dependency for
`StreamExt`. `EventStream` is a `Stream<Item = io::Result<Event>>`; its features
add `futures-core` and select Crossterm's level-polled Unix TTY source. The
default Mio source can return after either TTY input or SIGWINCH from one
edge-triggered readiness batch, leaving the other ready source unread without a
new edge. The level-polled source observes that unread source on the following
stream poll. The lockfile already contains futures 0.3.34 and the transitive
feature dependencies, so this changes feature edges without selecting a new
version.

Real-loop failure fixtures implement the existing macro-based Provider trait.
They use the already pinned workspace async-trait package as an app dev
dependency; this adds a test dependency edge without changing any version or
production interface.

Production constructs exactly one `EventStream`. A private generic loop entry
accepts `Stream<Item = io::Result<TerminalEvent>> + Unpin` so deterministic unit
fixtures can supply terminal events, EOF and errors without a native terminal.
No interactive path calls `event::poll` or `event::read` after stream creation.

### Four-source scheduler with deterministic rotation

Select among terminal events, the existing bounded typed-completion receiver,
the existing activity broadcast receiver and a persistent animation deadline.
A compact private scheduler cursor rotates the first branch in a biased
`tokio::select!`; after any wake the cursor advances to the following source.
Default `select!` randomization is fair statistically, but cannot support a
deterministic bounded-starvation assertion. Rotation guarantees that a source
which stays ready is served within one complete pass while adding no forwarding
task or queue.

Keep the animation deadline across unrelated wakeups so continuous activity or
typing cannot restart its wait. After the deadline fires, call the existing
elapsed-time animation function and schedule the next existing 25 ms busy or
100 ms idle probe. Runtime activity closure permanently disables only that select
branch; lag retains the current bounded omission notice.

### One ordered completion path with bounded activity projection

Both a completion queued before selection and one which wakes selection call one
handler. The handler first rejects a mismatched generation without changing the
current operation. For a current result it snapshots `Receiver::len`, caps the
snapshot at 256, and performs at most that many nonblocking activity receives.
Events sent during this drain cannot extend its captured receive budget, so a
producer cannot hold typed completion indefinitely. A broadcast lag notification
also consumes one receive from that budget; if older events were overwritten,
later events may be observed within the same bounded batch. The snapshot bounds
work rather than freezing an immutable set of event identities.

The handler then clears busy/job state; resolves pending quit from its typed
shutdown result; otherwise applies the exact command, turn or error outcome;
projects the runtime snapshot; settles; and redraws. This preserves
completion-before-refresh and failed-dispatch refresh plus settle. Activity keeps
its decorative role and never supplies completion text or status.

Explicit cancellation retains the current sequence: take and abort the job,
await it, drain only the bounded queued activity snapshot, advance the generation,
clear busy/quit state, settle, project runtime state, then publish the cancelled
status and completion lock. A raced send is stale before it can affect the next
turn. `/quit` still waits for its same-generation typed shutdown outcome.

### Common fallible-loop exit cleanup

Wrap the scheduling/rendering body in a private inner future while `run_loop`
retains the active `JoinHandle`. If the body returns an input EOF/error, draw
error, or completion-channel invariant error, the wrapper advances the generation,
aborts and awaits the active job, then returns the original contextual failure.
The outer `run` drops the EventStream and Ratatui terminal before its existing
`TerminalSession` restoration. This extends the TUI's existing task ownership to
all fallible loop exits; it does not change provider, Harness or dream semantics.

## Integration contract

Crossterm 0.29.0 exposes `event::EventStream` only under `event-stream`; its
stream item distinguishes event, `io::Error`, and end-of-stream. Its implementation
owns one native input helper thread and wakes that thread during `Drop`. Kuru owns
one stream for the terminal session and relies only on this documented stream
contract. Native PTY and ConPTY tests remain the authority for completed-frame,
EOF/error cleanup and actual console restoration behavior. A Unix PTY regression
must make resize and bracketed-paste input ready together while the child cannot
consume either event, then verify the same EventStream reports both without
requiring unrelated later input.

## Operational surface

The event stream runs only inside the local interactive `kuru` process against
its controlling stdin/stdout terminal. It opens no network bind address, adds no
container or runtime service, consumes no secret, and changes no provider
connection limit. The binary and architecture support matrix remains the
repository's existing native Unix and Windows x64/MSVC matrix; crossterm stays
exactly 0.29.0 and futures exactly 0.3.34. Native platform jobs, rather than
cross-compilation, establish EventStream and terminal-restoration behavior on
each supported OS.

## Risks / Trade-offs

- **Rotating biased select duplicates a small branch-order table.** Keep wake
  decoding in one helper and all state mutations in shared handlers; tests assert
  the rotation bound rather than each spelling of the table.
- **Activity may arrive while completion drains its preceding events.** Snapshot
  and cap the initial queue length; later activity is rendered afterward and
  cannot rewrite completed facts.
- **A generic stream fixture can prove scheduling but not native restoration.**
  Retain complete existing PTY/ConPTY bodies and exercise EOF/error through an
  actual terminal fixture on each supported native job.
- **EventStream uses an internal helper thread.** Construct and drop exactly one
  inside the terminal session, never mix readers, and verify process exit and
  terminal restoration after stream failure.
- **Unix resize and input may become ready in one native poll cycle.** Use
  Crossterm's supported level-polled TTY source and require a real PTY regression
  to observe both events without timing separation or a later wakeup.
