# Verification

## 1. Persisted replies carry part names [critical]

- [x] 1.1 @regression (agent) install a persisted page for a roster part and compare with the live label -> the entry reads the part name, not the UUID; before the fix it read the UUID. Observed before: a test calling the unchanged `Transcript::from_page` on a page whose speaker was `speaker` failed with `left: "speaker"`, `right: "Self"`. Observed after: `persisted_entries_use_the_resolver_and_keep_raw_speaker_ids`, `install_newest_preserves_the_live_label_and_older_pages_use_the_resolver` and `persisted_page_keeps_the_label_the_live_reply_showed` pass. Reverting only the `label(id)` call in `append_turn` makes 9 tests fail, including the store-backed one.
- [x] 1.2 @integration (agent) real managed store: complete two turns, reinstall the public page, restore the initial view -> `persisted_replies_show_the_part_name_not_its_uuid` (`runtime_tests.rs`) passes: live, reinstalled and restored replies all read the roster name, never the speaker id. `runtime_adapter_projects_real_relationship_completion_and_sanitized_route` now expects the roster name for the restored seeded reply.
- [x] 1.3 @unit (agent) older, newer, search and restore installs and the initial projection -> `older_newer_and_search_pages_resolve_part_names`, `persisted_page_keeps_the_label_the_live_reply_showed` (restore) and `initial_public_history_uses_stable_speakers_and_reports_omitted_turns` pass.

## 2. Roster changes and fallbacks

- [x] 2.1 @unit (agent) rename a part or switch framework after replies were installed -> `roster_and_relationship_changes_relabel_earlier_replies` and `relabel_changes_only_speaker_items_and_reports_no_change_when_stable` pass: earlier replies relabel, layout identity changes only for changed items. `unknown_speakers_share_one_prefix_fallback_and_system_roles_are_unchanged` shows live and reloaded entries share the 16-character prefix, `unknown` stays for a missing speaker and the interruption record stays `kuru`.
- [x] 2.2 @manual (agent) read record-level outputs (`kuru sessions export`, `/export`, boundary picker, `run --json`, `kuru memory export`) by source -> none go through the transcript projection; each still prints the stored speaker id and this change does not alter them. `/export` and `kuru sessions export` serialize `PublicTranscriptEntry` records (`speaker_id`); `kuru sessions` lists `last_completed_speaker`; the fork-boundary picker label embeds `record.speaker_id`; `run --json` prints `TurnOutput.speaker`; `kuru memory export` writes `StorageRecord::PublicTurn`. Only the display-name snapshot follow-on would change them.

## 3. Repository gates

- [~] 3.1 @integration (agent) full PR CI on the final head including the 95% workspace coverage gate -> defer: run 39449472581 at 24118cda passes every coverage partition and merge on all native platforms; only RustSec advisories (job 118720117618) is red, from advisories published 2026-10-14 against transitive rustls and zip that a separate dependency fix on main addresses. Re-observe on the rebased head before archive.

