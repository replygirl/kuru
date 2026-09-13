## 1. Confirmed managed-project removal [critical]

- [x] 1.1 @integration (agent) create a real Dolt project with committed rows and history, confirm purge, and inspect the managed directory/revisions -> `store::purge::tests::explicit_purge_removes_one_project_history_and_suppresses_legacy_reimport` passed against isolated real Dolt (exit 0), `/private/tmp/kuru-project-purge-memory-purge-tests-final.log`; it removes the selected managed directory and its revision store, then a fresh open has no imported state.
- [ ] 1.2 @e2e (agent) run `kuru memory purge --yes` against isolated project data with legacy SQLite, another project, export, engine, and shared-legacy sentinels -> only the selected managed project is removed and its next ordinary open does not resurrect legacy rows.

## 2. Ownership and retry safety [critical]

- [x] 2.1 @integration (agent) hold the actual lifecycle owner and invoke purge -> `store::purge::tests::live_owner_refusal_leaves_no_purge_control_or_tree_change` passed against an isolated real-Dolt owner (exit 0), `/private/tmp/kuru-project-purge-memory-purge-tests-final.log`; refusal leaves both activation and control absent and kills no process.
- [x] 2.2 @integration (agent) interrupt after durable intent and after checked quarantine move, replace original roots, then retry -> `reconstructed_pending_purge_blocks_open_then_resumes_only_its_recorded_tree` reconstructs a durable active-path intent and a later exact quarantined tree with its `ready.json` removed; each retry blocks ordinary open and removes only the recorded identity. `pending_purge_refuses_a_replacement_root_without_deleting_it` retains both replacement and parked original. All passed in `/private/tmp/kuru-project-purge-memory-purge-tests-final.log` (exit 0). These are reconstructed durable states, not a killed-process claim.
- [ ] 2.3 @integration (agent) present symlink/reparse and held-handle removal cases -> Unix symlink/replacement and cross-host success/Pinned-root fixtures passed in `/private/tmp/kuru-project-purge-platform-tree-test-final.log` (exit 0). Windows reparse-target preservation and held-root partial-removal/`Uncertain` fixture definitions compile in `/private/tmp/kuru-project-purge-platform-windows-typecheck-final.log` (exit 0), but native Windows execution remains pending.

## 3. User boundary and application cleanup

- [x] 3.1 @e2e (agent) inspect `kuru memory purge --help`, refusal without `--yes`, successful stdout/stderr, and canonical diagnostics cleanup -> isolated `cli_requires_explicit_project_purge_confirmation_and_removes_its_diagnostics_ring` passed (exit 0), `/private/tmp/kuru-project-purge-tui-cli-test-final.log`; help names retained legacy/snapshot/export boundaries, diagnostics cleanup, and recorded-identity retry, while both refusal and completion use `responses` with `OPENAI_API_KEY` removed. Successful stdout stays JSON, stderr stays empty, and the selected ring is absent.
- [x] 3.2 @integration (agent) run owning docs and focused static checks -> `mise run docs:check` passed (exit 0), `/private/tmp/kuru-project-purge-docs-check-final.log`; package typecheck/lint logs and the focused real-Dolt, CLI, and filesystem fixtures passed. Curated docs state retained shared/original copies, diagnostics cleanup, incomplete retry, no secure erasure, and no automatic expiry.

## 4. Native and coordinated evidence

- [ ] 4.1 @integration (agent) run native Windows checked-tree and project-purge fixtures -> held-handle, reparse, and uncertain removal behavior are observed natively.
- [ ] 4.2 @integration (agent) run coordinated workspace coverage -> project purge contributes to the required workspace coverage result.
