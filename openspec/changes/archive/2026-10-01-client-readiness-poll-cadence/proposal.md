# Proposal

## Why

A starting client waits for the memory service it just spawned by polling
`try_attach` in `attach_or_spawn_elected`'s readiness loop
(`packages/kuru-memory/src/service.rs`), sleeping a literal 100 ms after each
miss. The owner publishes its endpoint record only after binding its listener,
so once the record appears the next poll attaches; the open therefore waits, on
average, half a poll interval after the owner is actually ready. Measured on a
macOS release build at `1c93476f` (10 interleaved cold existing-project
samples), the stage from the endpoint record appearing to the client's `ready`
marker is 39 ms median (p90 61 ms) and shows a 100 ms comb; on Ubuntu (a second
harness at the same HEAD) the same stage is 57-64 ms median. That latency is
pure poll quantisation, paid on every cold open.

## What Changes

- The readiness loop sleeps `READINESS_POLL_INTERVAL` (10 ms), one named
  constant documented as a cadence, not a deadline. `startup_timeout_secs`, the
  readiness deadline and every other deadline keep their meaning and value.
- 10 ms is chosen against the cost of a failed attempt: before the record
  appears a miss is one small private-file read, a non-blocking child status
  check and, on observed opens only, one activity-record read. A Unix connect
  to a record without a listener is refused at once. A Windows named-pipe
  connect already retries `NotFound` and busy instances inside one attempt
  every 5 ms (`kuru-platform` `windows/pipe.rs` `POLL_INTERVAL`) until its own
  deadline; attempts are serial, so the outer cadence only adds a sleep after
  that connect returns and, at twice the pipe's interval, never polls finer
  than the platform's own retry.
- The owner-probe loop, election loop and read-only inspection loop keep their
  100 ms sleeps: they use separate literals, not a shared constant, and the
  owner-probe cadence also paces the project-ownership wait.
- Observable behaviour is otherwise identical: the readiness split's text and
  fields are unchanged; only its reported poll count rises (about ten times as
  many polls for the same readiness time). Owner stages forwarded between polls
  are now observed within about 10 ms instead of 100 ms; the memory guide's
  sentence about a short wait never being shown is reworded to name both ticks.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

(none; no requirement names the poll interval)

## Benchmarks

| Metric | Before | After | How measured |
|---|---|---|---|
| Endpoint record appeared → client `ready` (macOS, median / p90) | 39 ms / 61 ms | not re-measured (needs the owner-side timeline instrument) | Lead's measurement at `1c93476f`, release build, 10 interleaved samples |
| Same stage, Ubuntu median | 57-64 ms | not measured (Ubuntu) | Second harness at `1c93476f` |
| `open-start` → `ready`, cold existing-project open, macOS (median of 10) | 521.5 ms (min 518.6, max 523.6) | 446.1 ms (min 440.2, max 470.6) | Release build, isolated scratch data dir, `KURU_OPEN_MARKERS=1`, previous owner awaited between runs |

## Impact

- `packages/kuru-memory/src/service.rs`: `READINESS_POLL_INTERVAL`, the
  readiness loop's sleep, a `cfg(test)` per-poll hook, the cadence test and the
  stalled-owner poll-count plausibility check.
- `docs/memory.md`: the progress-sentence tick wording.
- No public API, protocol, configuration or spec text changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
