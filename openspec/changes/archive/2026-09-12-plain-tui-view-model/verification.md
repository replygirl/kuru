## 1. Plain view boundary [critical]

- [x] 1.1 @unit (agent) construct `View` synchronously from owned inputs, accept a completion and apply a different mutable snapshot -> `applying_runtime_snapshot_preserves_initial_editor_and_completion_state` passed: initial transcript/session/project/motion, editor draft/cursor and per-answer metadata survive while turns, selections and topology change
- [x] 1.2 @regression (agent) run plain UI, scene and visual fixtures and inspect their construction paths -> the non-async fixtures use owned Framework data and start no Harness or MemoryStore; View construction/application perform no history, environment or database reads. Package-owned bundle/prefetch preparation still ran separately before the test executables
- [x] 1.3 @equivalence (agent) run the restored characterization bodies with plain fixtures -> all 10 original plain UI cases, four scene cases and 10 nonignored visual cases passed. Root and Sol reviewed body-level preservation; the existing machine-dependent frame profiler remains intentionally ignored and no timing result is claimed

## 2. Runtime adapter truth [critical]

- [x] 2.1 @integration (agent) project initial and mutable data from a real deterministic Harness after a real history-seeding turn -> the adapter test passed full history, session/project, turns, selections, exact active labels, relationship and focus assertions against their runtime sources
- [x] 2.2 @integration (agent) run the retained real-Harness relationship and peer-route scenario in `tests/ui_runtime.rs` -> returned text and relationship speaker, exact returned input/output token numbers, relationship members and sanitized route render correctly; private peer text stays absent
- [x] 2.3 @integration (agent) run the original full slash-command scenario in `ui::runtime_tests` -> real mode/model/effort, focus, relationship, dream/undo, memory status/history and invalid-command checks passed through private dispatch

## 3. Completion and cancellation order [critical]

- [x] 3.1 @regression (agent) exercise typed completion, mutable snapshot application and late activity -> the new snapshot preservation case plus restored authoritative-completion/metadata/late-activity cases passed; independent source review confirms completion remains before snapshot application in run_loop
- [x] 3.2 @regression (agent) exercise preference feedback and failed dispatch through the TUI completion path -> the real terminal selections/restart/picker/failed-database-write fixture and retained slash validation case passed in coverage; root and Sol source review confirms feedback/error installation still precedes mutable projection and settle on every accepted outcome
- [x] 3.3 @e2e (agent) run the real delayed-provider cancellation fixture and next turn -> `real_pty_cancels_provider_work_preserves_draft_and_accepts_the_next_turn` passed; source review also confirms unchanged abort/await, drain, generation, settle, snapshot and cancelled-status order

## 4. Existing integration coverage

- [x] 4.1 @equivalence (agent) run complete TUI behavioral targets in the sole isolated workspace coverage task -> native macOS CLI, preferences, real PTY cancellation/selections, trust (14 cases), real adapter, nonignored visual cases and embedded offline installation/update passed with the unchanged committed memory implementation
- [~] 4.2 @e2e (human) run native Windows terminal and cancellation targets in native CI -> defer: no native Windows runner was available locally; ConPTY, persisted selections and cleanup remain required CI evidence before merge and are not inferred from macOS or compilation
- [x] 4.3 @integration (agent) run formatting, affected-package lint/typecheck, strict validation and coordinated coverage -> all passed with explicit exits; fresh isolated coverage is 17,656/18,286 lines (96.55 percent), with no concurrent coverage writer

## Observed evidence

2026-09-12, native macOS. Final source is tested in the isolated
`/private/tmp/kuru-phase0-tui-verify` worktree based on `add848e`, with exactly
the five owned TUI source/test files mirrored byte-for-byte to the implementation
worktree. It contains no uncommitted ordered-migration implementation.

Final restored UI cases passed 11/11 (the ten original cases plus the new
snapshot-boundary case). The real adapter and restored full slash case each
passed. Their private logs are `/private/tmp/kuru-tui-view-restore-*-final.log`
(ui, runtime, slash), each with explicit `EXIT=0`; root checked these results.
Affected-package lint/typecheck also exited 0. Scene and visual source remained
unchanged from their passing exact-name runs in
`/private/tmp/kuru-tui-view-verify-scene-final.log` and
`/private/tmp/kuru-tui-view-verify-visual-final.log`.

An earlier target-selection command unexpectedly selected the full TUI suite;
it was interrupted with exit 130 and is not completion evidence. A subsequent
read-only process check found no remaining owned fixture processes. Earlier
drafts that dropped assertions were rejected and restored before this checkpoint.
The single isolated `mise run coverage` finished with explicit exit 0 (session
72070), recorded in `/private/tmp/kuru-tui-view-coverage-final.log`, after
244.39 seconds. Root independently checked the completed log and fresh LCOV:
17,656/18,286 lines (96.55 percent), including the current UI and scene sources.
Integration test files do not appear as separate source records, but the log
records the real adapter and terminal cases passing. This verifies the TUI
change against committed `add848e` memory, not the uncommitted migration runner.

Final repository `mise run format:check` passed with explicit exit 0 (session
18953), logged in `/private/tmp/kuru-tui-view-format-check-final.log`. Its first
attempt found formatting-only changes in the separately held memory draft;
those owned paths were formatted before the successful rerun. Strict change
validation passed. Native Windows remains deferred as stated above.
