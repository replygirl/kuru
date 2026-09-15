## 1. Semantic output

- [x] 1.1 Add one fail-closed text/structured event projection path and apply it before broadcast, trace, journal writes and legacy replay.
- [x] 1.2 Replace only response-event answer detail with a completion marker while preserving safe state/peer semantics.
- [x] 1.3 Add distinct tool-call, peer-round, empty-response and legacy-unspecified output outcomes while keeping `limited` compatible.

## 2. Durable retry and interruption

- [x] 2.1 Persist one session-scoped local last-submission tuple atomically on new admission and expose controlled exact retry with completed-reuse status.
- [x] 2.2 Commit one deduplicated fixed interruption transcript marker with journal state, exclude it from provider context and preserve completed-answer race authority.
- [x] 2.3 Wire `run --turn-id` and TUI `/retry` without duplicate durable or displayed rows.

## 3. Diagnostics and documentation

- [x] 3.1 Print the checked debug-ring directory on stderr only when debug is active.
- [x] 3.2 Document retry, interruption, ring location and no-expiry proportional journal growth without conflating operational and semantic data.

## 4. Regression coverage and verification

- [x] 4.1 Add regressions for event leakage, legacy projection, distinct outcomes, safe/unsafe/completed retry, interruption persistence/deduplication and completion races.
- [x] 4.2 Run focused runtime/TUI/Dolt/PTY/docs and static checks, then record only observed evidence.
