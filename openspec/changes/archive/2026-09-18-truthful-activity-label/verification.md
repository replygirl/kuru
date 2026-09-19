## 1. The activity line is truthful while a tool call streams [critical]

- [x] 1.1 @regression (agent) `cargo test -p kuru --test terminal --all-features -- real_pty_activity_line_tracks_a_streaming_tool_call --exact`, first with `packages/kuru-runtime/src/progress.rs` and `apps/kuru-tui/src/ui/render.rs` reverted to their pre-change content (`git stash` of exactly those two files; the tests and the `View::calling_tool` field kept) -> FAILS, with the real-PTY frame showing `"activity · Responding"` while the tool call is in flight. Then restored (`git stash pop`) -> PASSES.
- [x] 1.2 @e2e (agent) same test: the held-open SSE fixture streams a `response.function_call_arguments.delta` and blocks; the frame is asserted with `wait_composer_frame(&["activity · Calling tool", "VISIBLE_SUMMARY"])`, then the screen is checked for the absence of `"Responding"` -> PASSES at 24x80; verbatim preview lines: `"│ thinking · VISIBLE_SUMMARY                                                   │"` and `"│ activity · Calling tool                                                      │"` (reverted run instead showed `"│ activity · Responding                                                        │"`).
- [x] 1.3 @unit (agent) `cargo test -p kuru-runtime --all-features --lib progress_tests::tool_loop_replaces_preview_and_partial_call_has_no_authority` -> the round-1 preview's `activity` is `"Calling tool"` (was `"Responding"`), round 2 is back to `"Responding"`: proves `ToolCallDelta` is observed, not dropped.
- [x] 1.4 @unit (agent) `cargo test -p kuru --lib --all-features facing_tool_call_names_the_activity_and_clears_on_settlement` -> a facing `Event::ToolStarted` renders `"activity · Calling state_report"` in a drawn frame; a peer's `ToolStarted` does not; `ToolSettled` and `begin_operation` clear it -> PASSES; verbatim drawn row at 120x35: `"│ activity · Calling state_report                                                                                      │"`.

## 2. Invariants preserved

- [x] 2.1 @unit (agent) the same real-PTY test asserts the streamed argument sentinel (`PRIVATE_ARGUMENT_SENTINEL`) and the key `activation` appear neither on the rendered screen nor anywhere in the PTY byte output; the TUI unit test asserts the same for the dispatched call's arguments -> PASSES: tool arguments are never rendered.
- [x] 2.2 @unit (agent) `packages/kuru-runtime/src/progress_tests.rs::relationship_consultation_and_dream_never_publish_private_streams` and `selected_part_and_relationship_only_preview_their_speaking_requests` pass unchanged -> private deliberation still never streams and reasoning is still not persisted (no `summary_tail` or persistence path touched).

## 3. No regression in the existing suites

- [x] 3.1 @e2e (agent) `cargo test -p kuru --test terminal --all-features --no-fail-fast` (full real-PTY file, including the neighbouring streaming/cancel fixtures) -> 19 passed, 0 failed, 1 pre-existing ignored; the neighbouring streaming/cancel/permission fixtures are unaffected.
- [x] 3.2 @unit (agent) `mise run //packages/kuru-runtime:test` -> 140 passed, 0 failed; `mise run //apps/kuru-tui:test` -> all binaries pass, 0 failed (1 pre-existing ignored).
- [x] 3.3 @integration (agent) `mise run format:check`, `mise run lint`, `mise run typecheck`, `mise run cospec:validate` -> pass.
