# Verification

## 1. A ready owner is attached within one finer poll [critical]

- [x] 1.1 @unit (agent) run `readiness_polls_every_interval_and_attaches_on_the_next_poll` under paused tokio time against a spawned owner that never publishes, with the test publishing a real listener and endpoint record after failed poll N -> consecutive failed polls are exactly `READINESS_POLL_INTERVAL` apart, the hook observes exactly N failed polls, and the client attaches to the published generation on poll N+1; observed passing 2026-10-01 on macOS, and failing with gaps of 100 ms when the old sleep was restored
- [x] 1.2 @benchmark (agent) release build, 10 cold existing-project opens per binary, interleaved, isolated scratch data directory, `KURU_OPEN_MARKERS=1`, previous owner awaited -> `open-start` → `ready` median 521.5 ms before (main `1c93476f`), 446.1 ms after; observed 2026-10-01 on macOS arm64

## 2. Behaviour and deadlines are unchanged

- [x] 2.1 @equivalence (agent) `mise run //packages/kuru-memory:test` -> the existing service, election, retirement and readiness-split tests pass; `readiness_split_text_is_stable` passes unchanged, and the stalled-owner tests still report `memory service readiness deadline exceeded` at the unchanged 1 s bound with a poll count consistent with the 10 ms cadence; observed exit 0 (500 passed, 0 failed) 2026-10-01 on macOS
- [x] 2.2 @regression (agent) `mise run format:check`, `mise run lint` (host and Windows target) and `mise run typecheck` -> clean exits; observed exit 0 for each, plus `//packages/kuru-memory:lint:windows`, 2026-10-01 on macOS
- [x] 2.3 @manual (agent) review the diff -> only the readiness loop's sleep changes cadence; `startup_timeout_secs`, the readiness deadline, `HANDSHAKE_TIMEOUT`, the election, owner-probe and inspection loops are untouched; observed in the committed diff

## 3. Windows

- [~] 3.1 @runtime (agent) native Windows run of the memory test suite -> defer: no local Windows host; CI's native Windows jobs run it on the PR
