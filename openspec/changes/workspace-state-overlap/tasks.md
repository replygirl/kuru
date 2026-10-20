# Tasks

## 1. Early refusal and classification

- [x] 1.1 Add the read-only nearest-existing-ancestor identity check and refuse every command after workspace retention except `config` and `trust status`, with one bounded message naming both directories and the remedies; verify by child-process tests for `sessions`, `trust approve`, `logout`, `run` (exit 2) and `--trust-workspace-once`.
- [x] 1.2 Regression: interactive TUI startup with the data directory inside the workspace (explicit `--data-dir` and default `XDG_DATA_HOME` derivation, with and without automatic claims) shows no trust prompt and no alternate screen; verify the real-PTY test fails on main (prompt shown) and passes after the fix.
- [x] 1.3 Add `ApprovalState::InsideWorkspace` for present and absent data directories, keep `Invalid` for malformed records, and print the distinct `trust status` wording; verify by trust.rs unit tests and a child-process `trust status` test.
- [x] 1.4 Report `state_inside_workspace`/`separate_state_directory` in doctor text and JSON for Workspace trust, ChatGPT subscription and memory with exit 3, independent of manifest claims; verify by doctor child-process tests.
- [x] 1.5 Remove the duplicate memory containment checks and keep the existing nested-data `config`/`sessions` test passing.

## 2. Documentation and checks

- [x] 2.1 Document the workspace/data-directory rule, status wording and doctor condition in docs/install.md, docs/configuration.md and the docs app; verify with `mise run docs:check`.
- [ ] 2.2 Run format, host and Windows lint, typecheck, app tests, cospec validation and managed checks; record observed results and CI coverage for the PR head.

Observed locally (macOS arm64, private `KURU_DOLT_CACHE`; the shared temporary engine cache was found empty and left untouched): before the fix on b1339bdc, `real_pty_launch_refuses_contained_state_without_the_trust_prompt` timed out with the screen at "Status: not approved … [1] Continue once  [2] Approve this complete configuration  [3] Cancel / Choice [3]:" and `a_workspace_containing_the_data_directory_is_refused_before_review` failed on `trust status` printing "Status: not approved" for absent nested data. After the fix both pass, with the trust.rs unit tests `approval_storage_inside_the_workspace_has_its_own_state` and `an_uninspectable_data_path_is_left_to_the_record_read`, doctor `doctor_names_a_workspace_that_contains_the_data_directory`, and the unchanged `configuration_inspection_ignores_tool_root_storage_but_activation_rejects_it`. `mise run //apps/kuru-tui:test` 447 passed, 0 failed. `format:check`, `lint:rust`, `lint:windows`, `typecheck`, `docs:check`, `cospec:validate` and `cospec:managed:check` exit 0. Task 2.2 awaits PR CI and coverage.
