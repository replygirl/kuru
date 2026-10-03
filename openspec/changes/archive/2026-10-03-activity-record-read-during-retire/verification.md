# Verification

## 1. The close settles the record before the endpoint retires [critical]

- [x] 1.1 @regression (agent) `service::activity::tests::the_publishers_last_write_lands_before_the_endpoint_retires`: every record write is gated until `AfterListenerDrop`; the pause is released, then one permit is added -> observed 2026-10-03 on macOS arm64. With the `finish_writes` call removed from `close_paused`, the test FAILED in 6 of 6 runs: `the endpoint retired while the publisher was writing`. With the fix it passed in 5 of 5 activity-suite runs.
- [x] 1.2 @integration (agent) `the_record_is_retired_after_the_endpoint_and_before_the_store_closes`, now waiting for `ended()` and requiring the record at `AfterEndpointRetire` to equal the settled record, with the read error reported -> observed: passed in 5 of 5 runs (macOS arm64)
- [~] 1.3 @regression (agent) the same tests natively on Windows (windows-latest, windows-11-arm) -> defer: needs native Windows, which runs in PR CI only; compiled and clippy-clean for the Windows target through `mise run lint:windows` (exit 0). The original Windows failure is attributed to the unawaited last write by inference, because the old test discarded the read error.

## 2. Marks and retirements unchanged

- [x] 2.1 @integration (agent) `mise run //packages/kuru-memory:test` (includes `a_failed_endpoint_publication_marks_then_retires_the_record`, `a_serve_error_before_the_starter_marks_then_retires_the_record`, `a_healthy_owner_keeps_its_record_until_its_starter_attaches`, `a_starter_meeting_a_closing_owner_waits_then_starts_its_own` and the service.rs `AfterEndpointRetire` pause tests) -> observed: exit 0; lib 699 passed, 0 failed, 6 ignored; the other binaries 10, 5, 12 and 1 passed. That run preceded only the removal of the now-unused `settled()` helper. On the final code, all 38 activity tests passed in 5 of 5 runs.

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check` -> observed: each exit 0 (format:check and lint run with `NODE_OPTIONS` unset)
- [x] 3.2 @unit (agent) `mise run cospec -- validate --all --strict` -> observed: 0 errors, 0 warnings; apply gate exit 0
