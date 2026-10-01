# Tasks

Work packages own disjoint files (reanchor section 6); work in `tmp/worktrees/feat-first-launch-feedback`, base `cb1c8e1d`. Commits C1-C5 map to WP-A to WP-E and each keeps the suite green. Tasks are ticked as evidence lands (see `verification.md` and `tmp/roadmap/unit2-implementation-notes-2026-09-30.md`).

## 1. Change artifacts

- [x] 1.1 Author proposal, blocking-changes, specs, design, tasks and verification, run `mise run cospec -- validate memory-open-activity --strict`, and verify it reports no error. Observed 2026-09-30 (macOS): `0 errors, 0 warnings — validation passed`.
- [x] 1.2 Run `mise run cospec -- apply memory-open-activity --json` and verify the exit code is 0, or that every soft blocker (exit 3) is resolved or acknowledged in writing. Observed 2026-09-30: gate state `clear`, exit 0, no blockers.

## 2. WP-A stages (`packages/kuru-memory/src/progress.rs`)

- [x] 2.1 Add `CreatingDatabase`, `UpgradingDatabase`, `StartingMemoryService` with bits, `ProgressReporter::is_observed()`, a const assertion that the variant count is at most 16, and the updated comment, and verify with `mise run //packages/kuru-memory:test` that the reporter's deduplication and capacity tests pass.

## 3. WP-B record, publisher, client read (`service.rs`, new `service/activity.rs`, `facade.rs`)

- [x] 3.1 Add the record codec with `activity_tag` and the `tag` field, and verify T9: round trip, unknown field, unknown stage name, over 4 KiB, wrong tag, and a record whose tag is the raw token is rejected.
- [x] 3.2 Add `Publisher` and `open_owner_store` reading `options.starter_token` (absorbing `open_owner_store_with_fixture_stages`, extending `fixture_startup_observations`), the two test-support hooks and their Windows forwarding, and verify T5 (publisher failure yields `[StartingMemoryService, Ready]`) and that an owner with `starter_token = None` publishes no record.
- [x] 3.3 Add `forward_new`, the `_observed` renames with silent wrappers, and the client stage reports at the owner-probe, after `let probed`, and in `attach_existing`, and verify T3, T4, T6 and T6b with really spawned owners, and that the deadline, `polls` and `ReadinessSplit` tests are unchanged.
- [x] 3.4 Add `ServiceOwner.activity`, `activity::retire` in `close_paused` after the `AfterEndpointRetire` pause and before `store.close()`, and the retire calls on both error returns of `ServiceOwner::open`, and verify T8 (revised: retire while held, the name is free at once) and T19 (waiting during the previous owner's reap, held at `AfterEndpointRetire`).
- [x] 3.5 Change the facade: delete the unconditional waiting report, call the `_observed` functions, add `open_local_forwarding`, and verify T7 (read-only fallback with an empty private `cache_dir` reports `ExtractingEmbeddedRuntime` before `Ready`) and that the six silent reattach callers compile unchanged.

## 4. WP-C CLI sentences, renderer, markers (`apps/kuru-tui/src/memory_activity.rs`, `cli.rs`, `lib.rs`)

- [x] 4.1 Add the constants S1-S5, N1, N2, N2' and `next_sentence` R1-R9, and verify T12 over the listed sequences and that no text contains `Memory:`.
- [x] 4.2 Move the renderer out of `cli.rs`, delete `memory_open_label` and its tests, add `interactive`, S1 at start, drain-then-render, width handling and the stdout rule, and verify T13 against a buffer (rewrite, truncation keeping the ellipsis, erase on complete and abandon, non-terminal lines, failing sink, N2 versus N2').
- [x] 4.3 Add the marker lines (anchor, `KURU_OPEN_MARKERS=1`, three events, terminal erase-marker-redraw) and verify the T13 marker cases: off by default, three events in order with non-decreasing ns, `waiting-ownership` only when R1 fired, no marker sharing a line with a sentence.

## 5. WP-E `store.rs` stages

- [x] 5.1 Insert `CreatingDatabase` and `UpgradingDatabase` at their logical sites, add `acquire_lock_reporting`, delete the unconditional waiting report, expose `released_v1`, delete `forwardable_before_store_stages` and its call sites, and flip `observed_open_reports_ready_only_after_a_usable_store`; verify T11, T2 and T1 with the `CreatingDatabase` hold.

## 6. WP-D child-process, PTY, docs (`apps/kuru-tui/tests`, docs)

- [x] 6.1 Replace `assert_memory_progress`, `expected_startup_line`, `assert_expected_startup_notice` and the `Memory:` block of `smoke`, and verify T14, T15, T16, T17 and T18, with T15 and T17's second run awaiting the previous owner's exit first.
- [x] 6.2 Add the marker child test (`KURU_OPEN_MARKERS=1 kuru run --json` on a new project, stderr not a terminal; and unset) and verify its assertions.
- [x] 6.3 Update `docs/memory.md` (five sentences, when each appears, terminal and non-terminal behaviour, nothing at ready, command output never receives it and stdout is used only for an interactive session with redirected stderr before the interface opens, the previous-owner wait), `docs/configuration.md`, `apps/kuru-docs/concepts/memory.md`, `apps/kuru-docs/reference/configuration.md`, the "startup notice when the engine was published" lines, and `docs/development.md` (markers and the two hooks only there); verify with `mise run docs:check` and `mise run docs:build`.

## 7. Integration and close-out

- [x] 7.1 Run `//packages/kuru-memory:test`, `//apps/kuru-tui:test`, `lint` (host and Windows target), `format:check`, `typecheck`, `docs:check`, `docs:build`, and verify each passes; name any not run and why. Observed 2026-09-30 on macOS: all pass; the Windows-target lint was last run at WP-D (no Rust changed since).
- [ ] 7.2 (deferred at archive: needs a release build and CI timing, not run locally) Record release-smoke medians for first launch and existing project against 6173 ms and 628 ms with each iteration awaiting the previous owner's exit, and verify the difference is within noise; Windows and PTY legs are native CI only.
- [x] 7.3 (PR opened; harness-owner and lead notices are orchestrator steps) Tell the open-time harness owner this change's PR number when it opens, record the changed frequency of the waiting sentence for the lead, then validate and run `mise run cospec -- archive memory-open-activity` before the final merge commit, and verify the archive exists.
