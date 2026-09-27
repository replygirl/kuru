# Tasks

## 1. Single release build per install job

- [x] 1.1 Rework `packages/kuru-memory/support/verify-bundle-build.ps1` (shipping command without `--all-features`, fresh-control step before the negatives, no trailing valid rebuild) and its task description, and verify the script parses and the local Cargo freshness signal behaves as the control expects
- [x] 1.2 Update the updater comment in `.github/workflows/native-tests.yml` and add the verifier/`build:release` parity test to `packages/kuru-delivery/tests/release_workflow.rs`, and verify with actionlint and `mise run //packages/kuru-delivery:test`
- [x] 1.3 Update `docs/development.md` and verify with `docs:check`

## 2. Verification

- [x] 2.1 Run `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check` and the local install dry run, and record observed results
- [x] 2.2 Put the before numbers from main run 36308781079 and a "Measured on this PR's CI" section in the PR body, and verify the Windows log lines listed in verification 1.1 there once the native run finishes

## Observed checks

Local checks ran on macOS arm64 in the branch worktree, 2026-09-27, from `origin/main` df42013f. That base commit was later removed from main; the change was re-applied unchanged onto 98b6b81a, and none of its files differ between the two bases.

- 1.1: verification 1.2-1.4 and 2.1. The control keeps the caller's `KURU_DOLT_BUNDLE_DIR` verbatim (including absent), because the build script tracks it and an absent-to-set change reruns it.
- 1.2: actionlint 1.7.12 clean; `mise run //packages/kuru-delivery:test` exited 0 (lib 162 passed; `release_workflow` 28 passed).
- 1.3: `docs:check` passed ("Public docs artifacts, local links and anchors passed").
- 2.1: the aggregate exited 0. `mise run install` (offline) exited 0 and the installed copy matches Cargo's output byte for byte.
- Re-applied onto 98b6b81a with the control's `Finished ` requirement: PowerShell 7.5.3 parser 0 errors; `lint:workflows` and the 2.1 aggregate exited 0; `mise run //packages/kuru-delivery:test` exited 0 (`release_workflow` 27 passed; the 28th test on the earlier base belonged to the removed df42013f).
- Native CI (verification 1.1 and 3.3), run 36316591798: the Windows job 108612335107 logged the fresh control (`Finished release in 1.14s`, matching SHA-256 `a509122b…`) and both negatives rejected at the kuru-memory build-script boundary; install jobs took Windows 13m26s (verify step about 10 s), Ubuntu 5m59s and macOS 5m28s, and the run 16m57s against main's 20m42s.
- Not run: `mise run //packages/kuru-memory:test` (only the memory task description and the Windows-only PowerShell script changed; no memory test reads either) and `mise run //apps/kuru-tui:test` (no install-path file changed). The real Windows execution of the verifier and the per-OS job times come from this PR's native CI run (verification 1.1 and 3.3). Coverage runs in the pre-push hook and CI.
