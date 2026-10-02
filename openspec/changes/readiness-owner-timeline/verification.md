# Verification

Authored 2026-10-02 before implementation; rows are ticked as evidence lands, with date and platform. Tests wait for events, never sleep. T4 and the end-to-end T5 are Unix-only (FIFOs; no never-connecting Windows supervisor fixture); Windows coverage is the stream unit tests, the pure accept classifier, the environment builder test and real CI owners in the gated coverage shard.

## 1. A readiness deadline names the owner event that consumed the budget [critical]

- [ ] 1.1 @unit (agent) T3 `owner_timeline_clause_text_is_stable`: synthetic stream files -> exact clause text; foreign tag ignored; partial last line ignored; `skipped=N`; `absent`, `empty`, `stale`, `unreadable`; ungated override -> no clause; the clause carries no path, scope or tag
- [ ] 1.2 @e2e (agent) T4 `readiness_deadline_names_the_owner_event_it_was_held_at`: real gated owner held at `create-start`, paused client clock runs to the default 30 s deadline -> leading text kept, `owner timeline:` with `owner-main`, `owner-lock`, `startup-lock`, `create-start` in order, `last=create-start`, an `owner-exec=` field, no `supervisor-spawned`, the `client phases:` split still parses; red on main at the inhibitor's bound
- [ ] 1.3 @regression (agent) the unchanged stalled-owner split tests (`readiness_deadline_reports_the_client_phase_split` and the two held-lock variants) and `readiness_split_text_is_stable` -> pass unchanged

## 2. The stream is create-only, private, bounded and gone before serving [critical]

- [ ] 2.1 @unit (agent) T1 `the_stream_mirrors_the_log_line_by_line` -> pre-attach entries flushed, one line per stamp in order, sealed/full log writes nothing, 64 longest names at `u64::MAX` within 8 KiB
- [ ] 2.2 @unit (agent) T2 `the_stream_is_create_only_private_and_removed` -> a taken name leaves streaming off with stamps still logged and the occupying file byte-identical and present; mode 0600; `end_stream` removes; only names and integers
- [ ] 2.3 @e2e (agent) T8 extended `a_gated_owner_writes_one_complete_record` (real child owner) -> new events in canonical subsequence, no `open-stream-*` file after publication

## 3. Unset changes nothing [critical]

- [ ] 3.1 @e2e (agent) `an_ungated_owner_writes_no_record` with `KURU_OPEN_TIMELINE=0` pinned -> no `open-timeline-*` or `open-stream-*` entry
- [ ] 3.2 @unit (agent) T3 ungated case -> no read and no clause even with the file present
- [ ] 3.3 @integration (agent) full `//packages/kuru-memory:test` with `KURU_OPEN_TIMELINE=1` exported (macOS) -> passes, including the listing tests `open_timeline_tests`, `usage_scan` `gated_opens_of_sealed_aged_stores_count_the_planned_rows` and `fixture_dir` scans

## 4. The supervisor deadline names its part

- [ ] 4.1 @integration (agent) T5 Unix `supervisor_readiness_deadline_names_its_part` (a supervisor that never answers, real clock at the 1 ms timeout plus the 2 s transport allowance; see design D8a) -> outer cause exact, inner names the Ready frame, child reaped (no cleanup-failure context)
- [ ] 4.2 @unit (agent) T5 pure `an_accept_timeout_at_the_deadline_is_the_accept_part` on every OS -> `TimedOut` at or after the deadline is the accept part; before it, or another kind, is not
- [ ] 4.3 @regression (agent) T7 extended `startup_log_capture_is_opt_in_exact_and_bounded` -> a split supervisor error still gets the fixture log

## 5. Windows forwarding and CI enablement [critical]

- [ ] 5.1 @unit (agent) T6 `the_owner_environment_forwards_only_an_exact_gate` (Windows) -> `1` forwarded, `0`/unset/`true` not, a test `0` overrides a forwarded `1`, no duplicate keys
- [ ] 5.2 @runtime (agent) the PR's first-attempt CI run with the gate in the coverage shard on Linux, macOS and Windows -> green; run id and one coverage job id per OS recorded
- [ ] 5.3 @integration (agent) `//apps/kuru-tui:test` with the env-clearing fixtures forwarding the gate -> passes, and `windows_cli.rs`'s shell-projection assertion is unchanged

## 6. Static checks

- [ ] 6.1 @regression (agent) `format:check`, `lint`, root `lint:windows`, `typecheck`, `docs:check`, `cospec:managed:check` -> exit 0, with no sleep, raised bound or retry added
