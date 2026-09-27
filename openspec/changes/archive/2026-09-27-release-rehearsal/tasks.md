# Tasks

## 1. Rehearsal mode

- [x] 1.1 Add the `mode` input, run name, concurrency group, per-job guards, rehearsal checkout refs, local stamps, credential check split and summary banner to `.github/workflows/release.yml`, and verify with pinned actionlint and the rendered publish-mode diff against `origin/main`
- [x] 1.2 Update `packages/kuru-delivery/tests/release_workflow.rs` for both modes and verify with `mise run //packages/kuru-delivery:test`
- [x] 1.3 Document the rehearsal in `docs/release.md`, adjust the single-entrypoint sentence in `AGENTS.md`, update the Windows arm64 status wording, and verify with `docs:check` and `cospec:validate`

## 2. Verification

- [x] 2.1 Run `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check` and record observed results

## Observed checks

All on macOS arm64 in the branch worktree, 2026-09-27, from `origin/main` 98b6b81a.

- 1.1: actionlint 1.7.12 exited 0. The publish-mode projection is byte-identical to main's `release.yml` and the rendered `needs`/`permissions`/`environment` graph is unchanged (verification 1.1). Rehearsal builds and staged checks use the dispatch SHA, which is `plan`'s `base_sha` (plan checks out `github.sha` and records `HEAD`), because no version commit exists; `build` and `verify-staged` run the bump job's `release:tool -- stamp` on their working tree so the binary version and `CARGO_PKG_VERSION` match the planned version as they would on the version commit.
- 1.2: `mise run //packages/kuru-delivery:test` exited 0 (lib 162 passed; `release_workflow` 27 passed including the new rehearsal test).
- 1.3: `docs/release.md` gains "Rehearse a release" and the publish dispatch instructions; `AGENTS.md` names the mode input. No in-repo document described Windows arm64 as "blocked"; the archived `windows-arm64-bundle-input` dependents list gains the from-source status (#113 input, #119 v2.3.4 pins, PR6b #115 in progress), and the gitignored `tmp/roadmap/delta-core.md` "Only Windows-on-ARM remains out" row was updated the same way.
- 2.1: `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check` exited 0 (docs: "Public docs artifacts, local links and anchors passed").
- Review fix (notes commit): the rehearsal-only notes commit now runs `git diff --quiet -- Cargo.toml Cargo.lock` and commits only when the stamp changed them, through `git add`, `write-tree`, `commit-tree -p HEAD` and `update-ref HEAD`, then always outputs `git rev-parse HEAD`. A HEAD already at the planned version (tagged, or last message `chore(release): v<current>`) previously made `git commit` exit 1; publish reuses that commit, and the rehearsal now does too. `core.hooksPath=/dev/null` did not disable hk 2.2.0's config-installed `hook.hk-pre-commit`/`hook.hk-commit-msg` hooks; plumbing runs neither. Observed in a scratch repository with both config hooks set to fail: a control `git commit` was blocked, the step script committed without running either hook, and a second run on the stamped HEAD made no commit and output the same SHA. `NOTES_COMMIT_STEP` and `docs/release.md` updated to match.
- Not run: the rehearsal dispatch itself (maintainer's decision; recorded as deferred verification 2.3), so the Windows `shell: bash` stamp step has not executed on a hosted Windows runner. Coverage runs in CI, not locally.
