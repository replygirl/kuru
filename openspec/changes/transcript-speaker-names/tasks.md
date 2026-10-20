# Tasks

## 1. Shared speaker labels

- [x] 1.1 Extract `speaker_label` from `View::actor_name` and keep every existing call site on it; verify existing live-label tests pass unchanged.
- [x] 1.2 Add `Item::speaker` and a label resolver parameter to `from_page`, `append_turn` and the page installs; verify persisted entries use the resolver and unknown ids use the 16-character prefix.
- [x] 1.3 Pass the view roster from `apply_public_read` (newest, restore, older, newer, search) and the initial view projection; verify no call site labels with a raw id.
- [x] 1.4 Add `Transcript::relabel` and call it from `apply_runtime` and `Event::Relationship`; verify a rename updates earlier replies.

## 2. Regression tests and checks

- [x] 2.1 Add transcript and view unit tests for live label preservation after `install_newest`, older-page labels, relabel on roster change, fallback consistency and restore; verify the persisted-label tests fail on the previous code.
- [x] 2.2 Add a real-store runtime test that re-reads a persisted turn and shows the part name; verify it passes.
- [x] 2.3 Run format, TUI tests, lint, Windows lint, typecheck, docs, cospec validate and managed checks; record observed outcomes.
- [ ] 2.4 Open the PR, record exact head SHA and CI run IDs, archive the change and confirm CI on the final head.

Observed (local, worktree at b1339bdc plus this change): format:check, lint:rust, lint:windows, typecheck, docs:check, cospec:validate and cospec:managed:check all exit 0. `mise run //apps/kuru-tui:test` exits 0 with 450 passed and 0 failed across all targets (the earlier run's single failure was the updated assertion below). One existing test, `runtime_adapter_projects_real_relationship_completion_and_sanitized_route` in `tests/ui_runtime.rs`, asserted the raw UUID as the restored label (the defect itself); it now expects the roster name.
