# Design

The approved research design with its critique and revision is `tmp/roadmap/readiness-owner-timeline-design-2026-10-02.md` (untracked shared notes); the lead's decisions L1-L4 of 2026-10-02 are binding and recorded below. Line numbers there are at `origin/main` `b1cf8a95`.

## Context

- The owner open timeline (`owner-open-timeline`, archived 2026-10-01) keeps one in-memory log per gated owner process and writes it only after close. On the readiness deadline path the starter has already bailed, so nothing is readable when it matters.
- The starter already holds the starter token it passes to its owner; `activity::activity_tag` is a keyed SHA-256 of it and already names the owner's activity record in the same owner-private `memory/services/<hash>/` directory.
- `fixture_startup_error` (`test_support.rs`) matches the supervisor-site cause `memory supervisor readiness deadline exceeded` by equality, and the stalled-owner parsers split the text after `client phases: ` on `; `. Both strings are therefore frozen; new text goes before the split or into an inner context.
- Windows owners are spawned with an explicit environment that does not carry the gate, and Windows spawn refuses duplicate case-insensitive keys; `kuru_platform::windows::process::merge_environment` already merges override layers.
- tokio 1.53.1 documents that a running `spawn_blocking` task inhibits paused-clock auto-advance on a current-thread runtime (`time/clock.rs`, `runtime/blocking/schedule.rs`).

## Goals / Non-Goals

**Goals:**
- The next A1 or supervisor-site occurrence names the owner phase, or the supervisor part, that consumed the budget.
- An end-to-end test holds a real owner at a named event and proves the clause names it.

**Non-Goals:**
- Any change to a wait, bound, retry, poll cadence, `startup_timeout_secs`, `SUPERVISOR_TRANSPORT_ALLOWANCE`, locks, reap order or the uncertain-write fence; removing engine starts. This is a diagnostic, not the fix.

## Decisions

- **D1 Stream file, not the activity record or stderr.** A separate create-only `open-stream-<tag>` file survives the starter's bail with one unsynced write per stamp. Rejected: adding stamps to `activity.json` (its `deny_unknown_fields` format 1 and staged, synced whole-record rewrites would change a product record for a diagnostic); owner stderr (null in the product); close-time-only events (unreadable at the deadline). The name does not start with `open-timeline-`, so existing listing filters and the close-time writer never see it.
- **D2 One gate seam.** `open_timeline::gate_set()` (exactly `1`) with a `cfg(test)` task-local override, so in-process starter tests are gated or ungated without touching the runner's environment. An ungated starter reads nothing (its deadline text is byte-for-byte unchanged).
- **D3 Bounded cost instead of "never delays" (L1).** The requirement states the cost: at most one unsynced write of at most 62 bytes per stamp plus one removal before serve, under the log mutex and the owner lock, never in close. Rejected: reading a syscall as "not a delay", which reads the requirement more loosely than written.
- **D4 Stale is detect-only (L3).** A same-tag file left by a killed predecessor (possible only with an explicit token) is refused by the create-only open and never appended to or removed; the starter prints `owner timeline: stale` when the file's `owner-main` wall-clock time precedes the instant taken immediately before `spawn_service`. Rejected: an owner removing a file it did not create, which widens its authority over the private directory.
- **D5 Deadline clause text.** `owner timeline: owner-exec=<ms>ms; <event>=<offset ms> … (ms); last=<event> +<ms>ms; since-last=<ms>ms[; skipped=<n>]`, or exactly `owner timeline: absent`, `empty`, `stale` or `unreadable`. `owner-exec` and `since-last` are wall-clock differences across two processes and are informative only. `empty` (a created file with no complete line) is an addition to the research design, which named only absent, stale and unreadable. A trailing partial line is ignored, malformed lines are counted, and names must match `[a-z0-9-]{1,32}`.
- **D6 Supervisor readiness part, one deadline (L2).** One `timeout_at` on the unchanged `startup_deadline`; a step cell (an `AtomicU8`, so the start future stays `Send`, where a `Cell` would not) records accept (Windows), startup write or Ready frame. On Windows the inner accept timer is kept; an accept error of kind `TimedOut` observed at or after the shared deadline is classified as the accept part and printed in the same shape with the unchanged outer cause, through one pure classifier compiled on every OS. Its root cause stays the accept's own timeout error rather than tokio's `Elapsed`, so the chain still shows which timer fired. Kind alone is not enough, since other errors map to `TimedOut`. Rejected: dropping the inner accept duration (literally a longer inner bound) and documenting two shapes (the phase could not be read from one text).
- **D7 FIFO event hold for tests, not `OPEN_HOLD_DIR_ENV`.** The existing stage hold gives no cross-process "entered" signal (the owner polls a marker file, so a test would have to poll), fires only at stage reports, and does not stop spawned tasks from stamping. The new test-support hold opens `<event>.entered` and `<event>.release` FIFOs, only in a gated owner, after the stamp's line is written and the log mutex released; its 300 s limit only stops a broken test from hanging an owner.
- **D8 T4 clock handling.** The end-to-end test does its real-clock setup first and calls `tokio::time::pause()` immediately before `attach_or_start` (rather than `start_paused`, under which runtime warm-up would auto-advance into its own timeouts). A `spawn_blocking` inhibitor waits for the `create-start` entered byte with a real-time bound of `startup_timeout_secs`: reaching `create-start` is a strict prefix of the open the product requires within that budget. The clock resumes the moment the inhibitor fails, so the red path never handshakes under a paused clock. Cleanup waits on a second hold at `endpoint-published`, whose release byte is written in advance, then attaches presenting the token so the owner closes at once.
- **D8a T5 clock handling.** The Unix end-to-end supervisor test runs on the real clock with the smallest startup timeout (1 ms), so its wait is the product's own deadline: that timeout plus `SUPERVISOR_TRANSPORT_ALLOWANCE` (2 s, `server.rs`). A paused clock would race the reap after the deadline: `finish_owner` polls `try_wait` with a virtual `sleep` bounded by `SUPERVISOR_REAP_ALLOWANCE`, which auto-advance would exhaust before the real `cat` stand-in exits. No sleep, bound or retry is added. The test asserts the reap through the absence of `memory startup cleanup also failed`.
- **D9 One change and PR (L4).** The supervisor split shares the phase table and docs with the owner clause.
- **D10 CI enablement.** `KURU_OPEN_TIMELINE = "1"` in the `coverage:shard` task environment only (identical for every partition), never in the usage-scan scaling job, which refuses it. Env-clearing kuru-tui fixtures forward it beside `LLVM_PROFILE_FILE` without changing what they assert about the child environment.

## Operational surface

The only deploy surface is CI execution: the `coverage:shard` task in `packages/kuru-delivery/mise.toml` gains `KURU_OPEN_TIMELINE = "1"` in its task environment, so every coverage partition on Linux, macOS and Windows runs gated owners and gated in-process starters. No workflow file, job topology, runner, container, secret, listener, bind address or connection limit changes; the usage-scan scaling job keeps refusing the variable. The owned, instrumented coverage supervisor and the pinned full-Dolt binaries per architecture are unchanged. The shipped binary's behaviour is unchanged unless a developer sets the variable; it then writes one extra owner-private file under the existing private data directory, removed before serving.

## Risks / Trade-offs

- [Gated owners do about forty small unsynced writes per open and one unlink before serving, which may perturb A1 rates] → Acceptance does not depend on rates; ungated product opens are unchanged.
- [Wall clocks across two processes can step] → `owner-exec`, `since-last` and `stale` are informative only; tests never assert their values.
- [A killed gated owner leaves its stream file] → Only in gated runs; a fresh product token is never read again; a reused explicit token prints `stale` and the successor streams nothing.
- [`absent` does not separate a slow owner exec from a stream-attach failure or an owner that published and removed its stream between the last poll and the read] → Documented in the developer docs' reading table; `owner-exec` separates the slow-exec case when present.
- [The real Windows double-timer accept path is not exercised end to end] → The pure classifier test runs on every OS; Windows streaming is exercised by the stream unit tests and real CI owners.
- [The test hold blocks one owner tokio worker while held] → Test-support only, gated owners only; no assertion depends on another owner task progressing.
