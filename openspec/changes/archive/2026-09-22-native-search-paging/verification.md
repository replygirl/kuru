## 1. Checked native search [critical]

- [x] 1.1 @integration (agent) run ToolHost grep/glob against a temporary checked project containing ordinary, ignored, hidden, protected, linked, and denied targets -> only eligible project-relative results are returned.
- [x] 1.2 @unit (agent) exercise bounds, invalid regex/glob inputs, Unicode, binary, and large-file fixtures -> failures are fixed/bounded and no result exceeds its documented cap.
- [x] 1.3 @eval (agent) inspect provider-visible grep/glob schemas and example receipts -> models receive bounded explicit search controls and never an ambient executable contract.

## 2. Stable paged file reads [critical]

- [x] 2.1 @integration (agent) call real ToolHost file_read for consecutive line pages in a Unicode text file -> the second request at next_offset starts at the immediately following logical line and unpaged text remains compatible.
- [x] 2.2 @unit (agent) exercise invalid ranges, binary input, and a file over the retained-output budget -> requests fail or project as documented without splitting UTF-8 metadata.

## 3. Published and installable contract

- [x] 3.1 @integration (agent) validate config schema against grep/glob native permission examples and run connector/core package tests -> parser and schema agree and native selector matching is covered.
- [x] 3.2 @integration (agent) build curated documentation -> tool reference describes paging, search defaults, permissions, and no external rg requirement.

## Observed evidence

- `mise run //packages/kuru-connectors:test` passed: 207 tests, including ToolHost search/paging fixtures.
- `mise run //packages/kuru-core:test` passed: 60 tests, including schema and permission parity.
- `mise run //packages/kuru-connectors:lint`, `mise run format:check`, `mise run docs:check`, and `git diff --check` passed.
- `mise run cospec -- validate native-search-paging --strict` passed with 0 errors and 0 warnings; independent final-delta source review cleared candidate authority, bounded snapshots, and spec wording.
- The first normal pre-push coverage attempt stopped during instrumented compilation because the shared volume was full; no coverage result was recorded. After merged-worktree cleanup, its replacement attempt exposed a missing TUI `NativeTool::Grep`/`Glob` prompt-label match before tests ran. The focused TUI test `ui::tests::native_search_grants_name_the_exact_candidate` and `mise run //apps/kuru-tui:typecheck` passed after adding candidate-scoped labels; an independent source review cleared that final delta.
