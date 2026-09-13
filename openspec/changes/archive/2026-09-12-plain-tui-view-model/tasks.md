## 1. Owned presentation seam

- [x] 1.1 In `apps/kuru-tui/src/ui.rs`, add owned `InitialViewData` and
  `RuntimeSnapshot` values with the existing transcript, part-label,
  relationship, and selection representations, then replace Harness-backed
  construction and refresh with synchronous `View` construction and runtime
  application.
- [x] 1.2 Add the TUI-local initial and mutable adapter projections so every
  Harness, history, project-path, and reduced-motion environment read remains
  outside `View`, and verify a real Harness projects every field used by the
  synchronous view.
- [x] 1.3 Update `run_loop` to project and apply owned snapshots while preserving
  completion-before-refresh, successful preference feedback, failed-dispatch
  refresh and settle, and the existing abort/await, activity-drain, generation,
  settle, refresh, notice, and cancellation-status order.

## 2. Plain and runtime test split

- [x] 2.1 Convert presentation-state tests in `apps/kuru-tui/src/ui.rs` to plain
  owned fixtures and verify completion metadata, late activity, topology
  filtering, pickers, editor behavior, and small-terminal rendering without a
  Harness or Dolt process.
- [x] 2.2 Convert `apps/kuru-tui/src/ui/scene.rs` tests to plain owned fixtures
  and preserve the existing static, relationship, route, compact-layout, and
  unknown-endpoint cell assertions without provisioning memory.
- [x] 2.3 Convert `apps/kuru-tui/tests/visual.rs` to plain owned fixtures and
  preserve the existing wide/narrow, framework, motion, editor, status,
  wrapping, picker, ambient, and frame-cost Ratatui assertions.
- [x] 2.4 Add `apps/kuru-tui/tests/ui_runtime.rs` as the explicit real-Harness
  projection target, retain the relationship/peer-route scenario there and move
  the real slash-command scenario into separate `src/ui/runtime_tests.rs`
  coverage of private dispatch, then verify production adapter output against
  actual runtime history, session, selections, topology, relationships, focus,
  project, returned completion, and sanitized routes so synthetic fixtures
  cannot replace runtime-truth coverage.

## 3. Behavioral equivalence and checks

- [x] 3.1 Run focused plain view, scene, visual, and real adapter-projection
  tests through the owning TUI mise task and verify plain targets start no Dolt
  process while the explicit integration target still exercises real memory and
  runtime behavior.
- [x] 3.2 Verify the complete TUI behavioral targets through the coordinated
  workspace coverage task, including existing PTY,
  cancellation, preferences, trust, persistence, provider, CLI, packaged
  runtime, and native Windows targets where available, and verify observable
  behavior remains unchanged; do not run a duplicate full TUI suite before
  the same tests under coverage.
- [x] 3.3 Run repository formatting, affected-package lint and typecheck,
  strict cospec validation, and one coordinated workspace coverage task; retain
  the bundled compile-time engine preparation and the 90-percent line gate.
