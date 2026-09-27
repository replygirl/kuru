# Tasks

## 1. Single release build per install job

- [x] 1.1 Rework `packages/kuru-memory/support/verify-bundle-build.ps1` (shipping command without `--all-features`, fresh-control step before the negatives, no trailing valid rebuild) and its task description, and verify the script parses and the local Cargo freshness signal behaves as the control expects
- [x] 1.2 Update the updater comment in `.github/workflows/native-tests.yml` and add the verifier/`build:release` parity test to `packages/kuru-delivery/tests/release_workflow.rs`, and verify with actionlint and `mise run //packages/kuru-delivery:test`
- [x] 1.3 Update `docs/development.md` and verify with `docs:check`

## 2. Verification

- [x] 2.1 Run `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check` and the local install dry run, and record observed results
- [x] 2.2 Put the before numbers from main run 36308781079 and a "Measured on this PR's CI" section in the PR body, and verify the Windows log lines listed in verification 1.1 there once the native run finishes

## Observed checks

All on macOS arm64 in the branch worktree, 2026-09-27, from `origin/main` df42013f.

- 1.1: verification 1.2-1.4 and 2.1. The control keeps the caller's `KURU_DOLT_BUNDLE_DIR` verbatim (including absent), because the build script tracks it and an absent-to-set change reruns it.
- 1.2: actionlint 1.7.12 clean; `mise run //packages/kuru-delivery:test` exited 0 (lib 162 passed; `release_workflow` 28 passed).
- 1.3: `docs:check` passed ("Public docs artifacts, local links and anchors passed").
- 2.1: the aggregate exited 0. `mise run install` (offline) exited 0 and the installed copy matches Cargo's output byte for byte.
- Not run: `mise run //packages/kuru-memory:test` (only the memory task description and the Windows-only PowerShell script changed; no memory test reads either) and `mise run //apps/kuru-tui:test` (no install-path file changed). The real Windows execution of the verifier and the per-OS job times come from this PR's native CI run (verification 1.1 and 3.3). Coverage runs in the pre-push hook and CI.
