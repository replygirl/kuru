# Verification

## Observed results

- `mise run //apps/kuru-tui:test -- composer` passed 6 composer unit tests, 2 terminal tests (including the new synchronized provider-fixture composer PTY, 1/1), and 3 visual tests. The PTY exercised Ctrl-R cancellation, history browse, 120-to-80-column resize, compact/expanded/removable paste chips, exact canonical provider input, and completed frames. The HTTP provider was deterministic and local; this is not live-provider evidence.
- Focused TUI tests passed 1/1 each for `submitted_history_is_ephemeral_session_owned_and_cancel_restores_draft`, `paste_chips_keep_canonical_crlf_and_remove_exact_combining_span` (including a joined ZWJ continuation), `oversized_paste_is_atomic_for_draft_and_chip_ranges`, `stored_scroll_clamps_after_transcript_and_viewport_shrink`, and `large_accepted_draft_renders_its_tail_with_cursor_visible` (70,000 lines within the accepted byte cap).
- `slash_completion_cycles_names_without_touching_arguments_or_modal_input` passed 1/1, including completion before a later chip and removal of only that chip, plus rejection at an exactly full 128 KiB draft without changing bytes, cursor or chip metadata.
- Existing routing regressions passed 1/1 each for `paste_does_not_mutate_draft_under_approval_or_picker_priority`, `session_picker_actions_preserve_drafts_and_use_exact_catalog_identities`, `permission_prompt_accepts_plain_digits_as_alt_digit_aliases`, and `instruction_review_keeps_permission_grants_distinct_and_disables_unsafe_persistence`.
- Existing synchronized `real_pty_shell_permission_cancel_closes_reply_and_preserves_draft` passed 1/1. The new composer PTY and this approval PTY use isolated fixtures and completed-frame synchronization.
- `every_store_opening_async_test_runs_its_body_in_the_closing_scope` passed 1/1 after the new async PTY fixture adopted the established closing wrapper. The U1 `/config` PTY and this guard also passed on the accepted U1 branch before U3 was based on its corrected head.
- `mise run //apps/kuru-tui:typecheck`, `mise run //apps/kuru-tui:lint`, and `mise run //apps/kuru-tui:lint:windows` passed on the final source. `mise run //:format:check` and `mise run //apps/kuru-docs:check` passed; the docs task built the site and verified public artifacts, local links and anchors.
- `mise run cospec -- validate phase2-composer-parity --strict` passed with zero errors/warnings. The actual `mise run cospec -- apply phase2-composer-parity --json` exited 0 with a clear gate, no hard blockers, and no soft acknowledgements.
- `mise run cospec -- archive phase2-composer-parity` exited 0 and created this archive at `openspec/changes/archive/2026-10-05-phase2-composer-parity/`; Cospec applied and verified four added requirements (`+4 ~0 -0 →0`).
- The full TUI/workspace suites, 90% workspace coverage gate, hosted CI, and live-provider calls were not run in this bounded local verification pass. The synchronized PTYs use deterministic local fixtures; hosted checks remain a delivery step after publication.

## 1. Grapheme-safe editing and exact submission [critical]

- [x] 1.1 @integration (agent) edit drafts containing combining marks, ZWJ emoji, CRLF, wide characters, and multiple lines through View key events -> each cursor/delete target is a grapheme boundary and canonical text matches the expected literal
- [x] 1.2 @e2e (agent) drive Unicode editing and submit through a real PTY at practical narrow and wide widths, synchronizing on completed frames -> cursor remains visible and the deterministic provider fixture receives the exact canonical draft

## 2. Bounded session history and Ctrl-R [critical]

- [x] 2.1 @integration (agent) force per-session, total-entry, and byte bounds; dispatch prompts under two session IDs; search, accept, and cancel -> eviction is visible, results stay session-scoped to dispatch ownership, and cancellation restores the exact draft/cursor without durable reads or writes
- [x] 2.2 @e2e (agent) browse and reverse-search history in a real PTY while an unsent draft exists -> acceptance inserts only the selected prompt and cancellation restores the draft without dispatching another prompt

## 3. Literal-preserving paste chips and atomic size bound [critical]

- [x] 3.1 @integration (agent) paste multiline Unicode, CRLF and terminal-looking text around surrounding draft text, then expand, edit, remove, and submit -> the chip projection never enters canonical prompt text, edits reveal literal spans, removal is exact, and a paste beyond 128 KiB changes nothing
- [x] 3.2 @e2e (agent) bracket-paste a large block into a real PTY and inspect completed compact and expanded frames before submission -> the deterministic provider fixture receives the exact accepted literal bytes and surrounding draft
- [x] 3.3 @integration (agent) paste a combining mark or ZWJ continuation adjacent to existing draft text and invoke chip controls from reachable grapheme-boundary positions -> cursor stays on whole graphemes while expansion and removal affect only the exact pasted byte span

## 4. Viewport and scroll clamping [critical]

- [x] 4.1 @integration (agent) render a long transcript and multiline composer while changing content and viewport bounds -> stored/effective scroll stays within current content limits and the composer cursor remains visible
- [x] 4.2 @e2e (agent) resize a real PTY across practical narrow and wide sizes during multiline editing and synchronize after each completed frame -> the visible cursor stays inside the input viewport and scroll returns to the current bottom without stale offsets

## 5. Modal priority and draft preservation [critical]

- [x] 5.1 @integration (agent) exercise picker, session rename, instruction review, permission approval, cancellation, and unavailable approval choices with an unsent draft and paste spans -> each modal retains input priority, preserves the draft/cursor/chips, and ordinary digits edit only after dismissal
- [x] 5.2 @e2e (agent) open and dismiss picker and approval overlays during a real PTY draft edit -> submitted prompt bytes and approval outcome reflect only the selected modal action and unchanged draft

## 6. Documentation and repository gates [critical]

- [x] 6.1 @integration (agent) run focused TUI checks, affected docs checks/build, format, lint, strict Cospec validation and the actual Cospec apply gate -> scoped local checks pass, the change is archive-ready, and unrun hosted/full-suite/live checks are named explicitly
