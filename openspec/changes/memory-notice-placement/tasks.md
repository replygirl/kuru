# Tasks

## 1. Anchored local transcript items

- [x] 1.1 Add a regression unit test that pushes `/help` output, submits and binds a turn, installs the persisted newest page and asserts the help output stays before the new user message; observe it fail on the unchanged `install_projection`, then pass.
- [x] 1.2 Record anchors at push and re-place locals by anchor across newest/restore/search/older/newer installs; count only public items in eviction, newer insertion, `cursor_for` and `next_match`; verify with unit tests for error lines, vanished anchors, bound items, mid-stream locals through eviction and local bound/omission accounting.

## 2. Memory notice as a status indicator

- [x] 2.1 Replace the transcript notice with a non-expiring status-line indicator recorded after its completed frame and cleared on key/paste; add the `/status` memory line, `kuru memory --help` reference and shortened headless line; verify with real-loop runtime tests (first frame, record ordering, clear on input, `/status`, not persisted, not in provider request, failed first draw).
- [x] 2.2 Update CLI, PTY and shell tests pinning the old wording, docs (`docs/usage.md`, `docs/memory.md`, `apps/kuru-docs`) and run format, TUI tests, host and Windows lint, typecheck, docs, cospec and managed checks.

Observed (local, macOS arm64): before the fix `help_output_keeps_its_place_when_the_next_turn_is_persisted` failed with `left: ["question-1", "answer-1", "question-2", "answer-2", "help output"]`; after it passes with the other eight `ui::transcript` tests. `mise run //apps/kuru-tui:test` exit 0 (450 passed, 6 ignored), including `first_run_indicator_is_drawn_before_input_then_recorded_outside_the_transcript`, `failed_initial_tui_draw_never_marks_the_notice_shown`, `notice_text_never_reaches_the_provider_request_for_a_real_tui_turn`, `memory_indicator_outlasts_transient_notices_and_clears_on_input`, `memory_long_help_lists_the_retention_controls` and the real-PTY suite (68 passed). `format:check`, `lint:rust`, `lint:windows`, `typecheck`, `docs:check`, `cospec:validate`, `cospec:managed:check` exit 0. Coverage is not run locally; its 95% gate runs in CI.

## 3. Delivery

- [ ] 3.1 Open the PR, observe CI green including the 95% coverage gate on the exact head, and record run IDs.
