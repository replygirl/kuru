# Tasks

## 1. Cadence

- [x] 1.1 Replace the readiness loop's literal 100 ms sleep with `READINESS_POLL_INTERVAL` (10 ms), commented as a cadence, not a deadline, and verify no other loop or deadline changes in the diff
- [x] 1.2 Reword `docs/memory.md`'s short-wait sentence to name the readiness and ownership-wait ticks and verify `mise run docs:check`

## 2. Tests

- [x] 2.1 Add a paused-time cadence test: consecutive failed polls are exactly `READINESS_POLL_INTERVAL` apart and an endpoint published after poll N is attached on poll N+1; verify it passes
- [x] 2.2 Update the stalled-owner poll-count plausibility check to the constant and verify the readiness split tests pass
- [x] 2.3 Run `mise run //packages/kuru-memory:test`, `mise run format:check`, `mise run lint` and `mise run typecheck` and verify each passes

## 3. Benchmark

- [x] 3.1 Measure 10 cold existing-project opens with a release build (`open-start` → `ready` median, before and after) and record the after number in the notes

## Observed evidence

Recorded 2026-10-01, macOS 27.0 arm64 (host load average 10-15 from other work).

- 1.1: one constant, `READINESS_POLL_INTERVAL` = 10 ms; the diff changes no other loop, deadline or timeout.
- 1.2: `mise run docs:check` exit 0.
- 2.1: `readiness_polls_every_interval_and_attaches_on_the_next_poll` passed; with the loop's sleep temporarily reverted to 100 ms it failed with `readiness polls were not 10ms apart: [100ms, 100ms, 100ms, 100ms]`.
- 2.2: the three stalled-owner split tests and `readiness_split_text_is_stable` passed.
- 2.3: `mise run //packages/kuru-memory:test` exit 0 (lib: 500 passed, 0 failed, 4 ignored; integration targets all ok); `mise run format:check`, `mise run lint`, `mise run //packages/kuru-memory:lint:windows` and `mise run typecheck` exit 0.
- 3.1: release builds, 10 interleaved cold existing-project opens each (order alternated per round, previous owner awaited, no ownership waits observed): `open-start` → `ready` median 521.5 ms before (`1c93476f` binary, sha256 `983e3cee…`) and 446.1 ms after (sha256 `6ecbed0c…`).
