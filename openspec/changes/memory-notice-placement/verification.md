# Verification

## 1. Local items keep their place [critical]

- [x] 1.1 @regression (agent) unit test: `/help` local, bound user/answer, `install_newest` of the persisted page -> observed failing on unchanged b1339bdc code (`["question-1", "answer-1", "question-2", "answer-2", "help output"]`); passes after: help stays after answer-1, before question-2, also after a further persisted turn.
- [x] 1.2 @unit (agent) transcript tests for error lines, vanished anchors, `None` anchors, bound duplicates, mid-stream locals through `install_older`/`install_newer` eviction, search cursor/match on public items, local bound/omission counts -> 9/9 `ui::transcript` tests pass locally.

## 2. Memory status indicator [critical]

- [x] 2.1 @integration (agent) real TUI loop over a real persistent store: first completed frame shows `Memory ready ·` in the status row and not in the transcript; marker recorded only after it; first key clears it; `/status` shows the escaped directory; reopen shows nothing pending; no stored message contains it -> `first_run_indicator_is_drawn_before_input_then_recorded_outside_the_transcript` passes on a frame-recording backend; `memory_indicator_outlasts_transient_notices_and_clears_on_input` covers the 5 s timer, precedence, focus/resize retention and key/paste clearing.
- [x] 2.2 @integration (agent) failed initial draw leaves the notice pending; a real turn's provider request contains no indicator text -> both runtime tests pass locally.
- [ ] 2.3 @e2e (agent) real-PTY smoke shows the indicator on first run and not on reopen; headless `kuru run` stderr line keeps directory, retention sentence and `kuru memory --help`; `kuru memory --help` lists the four controls -> pass in CI on supported platforms. Local macOS: terminal suite 68 passed, cli and unix_shell_turn suites pass; native CI pending.

## 3. Static and delivery checks

- [ ] 3.1 @integration (agent) format, host/Windows lint, typecheck, docs check, cospec validate and managed check; CI including the 95% coverage gate on the exact head -> pass.
