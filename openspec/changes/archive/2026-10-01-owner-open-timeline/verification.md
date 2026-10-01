# Verification

Authored 2026-10-01 before implementation; rows are ticked as evidence lands, with date and platform. Windows is out of scope for the instrument; Windows-only code is only compiled and linted. Tests wait for events, never sleep. The macOS measurement rows are not part of the archive gate (design D8) and are deferred with a reason until observed.

## 1. A gated owner's file appears only after its lock is released [critical]

- [x] 1.1 @integration (agent) test 6 `service/open_timeline_tests.rs`: in-process owner, pauses at `BeforeListenerDrop`, `AfterListenerDrop`, `AfterEndpointRetire`, `AfterReap` -> no `open-timeline-*` and the owner lock held at each pause; after `serve_with` exactly one file named for the generation and the lock free; Observed 2026-10-01 (macOS arm64): `the_record_appears_only_after_the_owner_lock_is_released` ok
- [x] 1.2 @e2e (agent) test 8 child owner with `KURU_OPEN_TIMELINE=1` on an existing store, awaited exit -> exactly one file, 18 canonical events in order, non-decreasing offsets, `dropped = 0`, `late = 0`, integer `usage_rows`, `format` and `format_version` as specified; Observed 2026-10-01 (macOS arm64): `a_gated_owner_writes_one_complete_record` ok
- [x] 1.3 @unit (agent) test 5 write -> mode `0600`, bytes equal `encode`, a second write of one generation returns `Err` and leaves the first file, a missing directory and a directory in the name's place return `Err`; Observed 2026-10-01 (macOS arm64): `the_write_creates_one_private_file_and_never_replaces_one`, `the_write_fails_without_its_directory_or_with_its_name_taken` ok

## 2. Unset changes nothing [critical]

- [x] 2.1 @e2e (agent) test 9 child owner without the variable -> ordinary exit and no `open-timeline-*` entry; Observed 2026-10-01 (macOS arm64): `an_ungated_owner_writes_no_record` ok
- [x] 2.2 @integration (agent) unmodified `a_starter_forwards_only_its_own_owners_stages`, the creation and upgrade siblings, and `cli_open_markers_are_written_to_standard_error_only_when_asked` -> pass unchanged; Observed 2026-10-01 (macOS arm64): the three activity tests ok in the full package suite; `//apps/kuru-tui:test -- cli_open_markers_are_written_to_standard_error_only_when_asked` 1 passed
- [x] 2.3 @unit (agent) test 3 gate -> true only for exactly `1`; false for unset, empty, `0`, `true`, ` 1`, `1 `, `11` and non-UTF-8; Observed 2026-10-01 (macOS arm64): `only_exactly_one_enables_the_timeline` ok

## 3. Recording and writing never fail or lengthen the open, serve or close [critical]

- [x] 3.1 @integration (agent) test 7 a directory occupying the file name -> `serve_with` returns `Ok`, endpoint retired, quiescence passes, lock free, the directory untouched; Observed 2026-10-01 (macOS arm64): `a_failed_record_write_leaves_the_close_unchanged` ok
- [x] 3.2 @unit (agent) test 1 four threads, four tasks and a blocking task stamping 10,000 times -> offsets non-decreasing, `entries + dropped = 10,000`, capacity unchanged; with capacity 64 and 200 stamps, 64 entries, `dropped = 136`, no panic; Observed 2026-10-01 (macOS arm64): `concurrent_stamps_stay_ordered_and_never_grow_the_log`, `a_full_log_counts_drops_without_growing` ok
- [x] 3.3 @unit (agent) test 2 stamps after `endpoint-published` -> not stored and `late` counts them; Observed 2026-10-01 (macOS arm64): `stamps_after_endpoint_publication_are_counted_late` ok
- [~] 3.4 @benchmark (agent) fresh-store A/B, 20 runs ABBA with the variable on and off, observer row Dolt endpoint first seen to service endpoint first seen (expects on-minus-off median inside the +/-2 ms bracket, otherwise the gap is reported as the instrument's cost) -> defer: the 20-run ABBA observer-row A/B was not run; only a 10-pair wall-time A/B was (on 1.512 s vs off 1.446 s median, noise under concurrent load), see the notes

## 4. The record carries no sensitive content

- [x] 4.1 @unit (agent) test 4 encode -> exact field set and format tag, canonical names, integer `ns` and `anchor_unix_ns`, `usage_rows` null versus set, a full 64-entry log under 8 KiB; Observed 2026-10-01 (macOS arm64): `the_record_has_the_specified_fields_and_names`, `a_full_log_encodes_within_its_bound`, `an_oversized_record_is_refused` ok
- [x] 4.2 @e2e (agent) test 8 byte search -> the file contains none of the data directory, project path, scope, scope hash, connection secret or store instance; Observed 2026-10-01 (macOS arm64): checked inside `a_gated_owner_writes_one_complete_record`, ok

## 5. The aged-store fixture is deterministic and uses the real write paths [critical]

- [x] 5.1 @integration (agent) test 10 two template stores aged with seed 7, 3 conversations, 2 turns -> equal reports matching writes `3*(2+8*2) = 54` (reconciled from the source design's `2+7T`: the runtime's possible-dispatch journal `put` is copied) and usage rows `3*(1+3*2) = 21`, equal session catalog ids and labels, equal ledger sessions and transcript windows; a different seed gives different session ids; Observed 2026-10-01 (macOS arm64): `equal_plans_age_equal_content_with_the_stated_counts` ok
- [x] 5.2 @integration (agent) test 10 wrapper (`aged_store::claim` then `age_claimed`, which `main` composes) on a real warmed data directory with seed 7, 2, 1 -> counts equal the formula the inner test checks for the inner function, and the owner lock is free afterwards; Observed 2026-10-01 (macOS arm64): `the_claimed_wrapper_ages_the_one_store_and_releases_its_lock` ok
- [x] 5.3 @unit (agent) argument parser -> required flags, defaults, and rejection of bad numbers and unknown flags; Observed 2026-10-01 (macOS arm64): `parse_reads_flags_and_defaults`, `parse_refuses_malformed_arguments`, `the_report_line_has_its_format_and_exact_fields`, `generated_content_is_a_pure_function_of_seed_and_index` ok

## 6. Static checks

- [x] 6.1 @regression (agent) `mise run //packages/kuru-memory:test`, `lint`, `lint:windows`, `format:check`, `typecheck` and `docs:check` -> all pass, with no sleep, raised deadline or retry added; coverage's 90% line gate is enforced in CI and is not run locally; Observed 2026-10-01 (macOS arm64) on the final tree rebased onto origin/main `ad743791` (#152), branch head `d7a05411` before this ledger edit: `//packages/kuru-memory:test` lib 521 passed, 0 failed, 4 ignored (the 520 of the pre-rebase tree plus #152's `readiness_polls_every_interval_and_attaches_on_the_next_poll`), and every other test target ok; `//packages/kuru-memory:lint`, `//packages/kuru-memory:lint:windows`, root `format:check`, `//packages/kuru-memory:typecheck` and root `docs:check` exit 0; the test run used an isolated owner-private `KURU_DOLT_CACHE` in the session scratchpad. An independent review run on the pre-rebase head `3470f13e` (base `1c93476f`) saw the same checks pass with 520 lib tests.
- [x] 6.2 @regression (agent) PR #154 CI failed only on both Windows runners in `parse_reads_flags_and_defaults` and `parse_refuses_malformed_arguments`, whose fixtures used `/d`, a path that is not absolute on Windows, so the parser's "--data-dir must be absolute" rule fired on valid input; fix: the tests build their absolute fixture paths from a per-platform root (`C:\` on Windows, `/` elsewhere) and the parser rule is unchanged, the relative-path refusal still asserted -> same assertions pass; Observed 2026-10-01 (macOS arm64): `//packages/kuru-memory:test -- --lib aged_store` 7 passed, root `lint`, `//packages/kuru-memory:lint:windows` (clippy `--all-targets` for x86_64-pc-windows-msvc compiles the corrected tests), `typecheck` and `format:check` exit 0; the Windows execution of the two tests is not verified locally and awaits the PR's Windows CI.

## 7. macOS split measurement (not an archive gate)

- [~] 7.1 @benchmark (agent) release build, fresh and aged stores at the pilot-chosen ladder, 10 interleaved samples each, expecting per-row min, median, p90, max and a growth ranking in `tmp/roadmap/unit6b-notes-2026-10-01.md`; not yet run -> defer: measurement work package 4, recorded in the notes rather than in this ledger
- [~] 7.2 @benchmark (agent) Ubuntu split, expecting the same rows; not yet run -> defer: comes later from assistant3's harness; not run here
