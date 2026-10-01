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
- [ ] 3.4 @benchmark (agent) fresh-store A/B, 20 runs ABBA with the variable on and off, observer row Dolt endpoint first seen to service endpoint first seen -> on-minus-off median inside the +/-2 ms bracket, otherwise the gap is reported as the instrument's cost; not yet run -> defer: needs the release build and the macOS series (work package 4)

## 4. The record carries no sensitive content

- [x] 4.1 @unit (agent) test 4 encode -> exact field set and format tag, canonical names, integer `ns` and `anchor_unix_ns`, `usage_rows` null versus set, a full 64-entry log under 8 KiB; Observed 2026-10-01 (macOS arm64): `the_record_has_the_specified_fields_and_names`, `a_full_log_encodes_within_its_bound`, `an_oversized_record_is_refused` ok
- [x] 4.2 @e2e (agent) test 8 byte search -> the file contains none of the data directory, project path, scope, scope hash, connection secret or store instance; Observed 2026-10-01 (macOS arm64): checked inside `a_gated_owner_writes_one_complete_record`, ok

## 5. The aged-store fixture is deterministic and uses the real write paths [critical]

- [ ] 5.1 @integration (agent) test 10 two template stores aged with seed 7, 3 conversations, 2 turns -> equal reports matching writes `3*(2+7*2) = 48` and usage rows `3*(1+3*2) = 21`, equal session catalog ids and labels, equal ledger sessions and transcript windows; a different seed gives different session ids
- [ ] 5.2 @integration (agent) test 10 wrapper `aged_store::run` on a real warmed data directory with seed 7, 2, 1 -> report equals the inner function's for the same plan on a sibling store, and the owner lock is free afterwards
- [ ] 5.3 @unit (agent) argument parser -> required flags, defaults, and rejection of bad numbers and unknown flags

## 6. Static checks

- [ ] 6.1 @regression (agent) `mise run //packages/kuru-memory:test`, `lint`, `lint:windows`, `format:check`, `typecheck` and `docs:check` -> all pass, with no sleep, raised deadline or retry added; coverage's 90% line gate is enforced in CI and is not run locally; open: the WP1 and docs state was observed passing on 2026-10-01 (macOS arm64: package test 514 passed, package `lint` and `lint:windows`, root `lint`, `format:check`, `typecheck`, `docs:check` exit 0), but WP2 has not landed, so this row waits for the final tree

## 7. macOS split measurement (not an archive gate)

- [~] 7.1 @benchmark (agent) release build, fresh and aged stores at the pilot-chosen ladder, 10 interleaved samples each, expecting per-row min, median, p90, max and a growth ranking in `tmp/roadmap/unit6b-notes-2026-10-01.md`; not yet run -> defer: measurement work package 4, recorded in the notes rather than in this ledger
- [~] 7.2 @benchmark (agent) Ubuntu split, expecting the same rows; not yet run -> defer: comes later from assistant3's harness; not run here
