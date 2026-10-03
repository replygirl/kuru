# Verification

## 1. A slow Git index check completes instead of aborting [critical]

- [x] 1.1 @regression (agent) `discovered_local_config_waits_for_a_slow_git_index_check`: real `kuru config` in a `.git` root with the `kuru-slow-git-fixture` binary copied onto PATH as `git` (sleeps 5.5 s, exits 1 = untracked) -> observed 2026-10-03 macOS arm64 via `mise run //apps/kuru-tui:test -- discovered_local_config` (name filter; the task's `--all-targets --all-features` still builds every kuru target and enables the `test-support` gate): with `cli.rs` reverted to the 5 s cap, FAILED (task exit 101) `panicked at apps/kuru-tui/tests/cli.rs:1723:5: Error: project-local Git index check timed out`; with the fix, `test discovered_local_config_waits_for_a_slow_git_index_check ... ok` (task exit 0, 3 passed in 8.11 s). The fixture is cross-platform (copied with `EXE_SUFFIX`); its Windows run happens in PR CI only, unrun locally

## 2. Designed behaviour unchanged

- [x] 2.1 @integration (agent) sibling tests `discovered_local_config_requires_untracked_git_provenance_and_preserves_repo_claims` (real Git: tracked rejected, untracked accepted, `GIT_*` redirection stripped) and `discovered_local_config_accepts_non_git_roots_and_rejects_ambiguous_index_status` -> observed: both ok in the same green run (and in the red run)

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> observed: each exit 0, run with the session's `NODE_OPTIONS` preload unset (format:check first failed on rustfmt layout of the new test, fixed with `cargo fmt -p kuru`, then exit 0); no docs name the removed message, so docs are unchanged
