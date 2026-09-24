## 1. Provider-delivered private record [critical]

- [x] 1.1 @integration (agent) drive a successful typed provider completion through the admitted runtime path -> one `reasoning_summary.v1` record has the real session, turn, actor, invocation, final item, output, and summary coordinates (observed 2026-09-22: prepared-Dolt runtime fixture 1/1)
- [x] 1.2 @regression (agent) retry or recover the same admitted settled item -> one idempotent private record, with no record for a failed or summary-free completion (observed 2026-09-22: local batch 2/2, managed lost reply 1/1, runtime exclusions 5/5, real-Dolt close/reopen 1/1)

## 2. Privacy and isolation [critical]

- [x] 2.1 @integration (agent) inspect interactive-facing progress, completed output, and public events while a provider emits a reasoning summary -> raw and settled private payloads and provider coordinates are absent (observed 2026-09-22: runtime progress non-disclosure 2/2)
- [x] 2.2 @integration (agent) retain a private summary in one session, then reopen memory and begin a new session -> the owner full-memory export retains the private row, while neither transcript history nor the new session's provider context receives it (observed 2026-09-22: real-Dolt reopen 1/1, cross-session provider-context isolation 1/1, full-memory export 1/1). P08 adds no session-export or fork presentation surface; later capabilities own the required no-sweep acceptance.
- [x] 2.3 @regression (agent) create equal-looking provider summaries across sessions, turns, and invocations -> exact identity lookup keeps each record isolated (observed 2026-09-22: identity fixture 1/1)

## 3. Provider behavior envelope

- [x] 3.1 @eval (agent) run bounded deterministic provider cases with a delivered summary, no summary, and terminal failure -> records appear only for delivered successful typed summaries and no request/prompt shape changes (observed 2026-09-22: connector 3/3; runtime 5/5)

## 4. Public documentation

- [x] 4.1 @manual (agent) build and inspect the relevant documentation page -> provider-delivered scope and privacy limits are stated without claiming transcript or cross-session availability (observed 2026-09-22: `//apps/kuru-docs:check`)
