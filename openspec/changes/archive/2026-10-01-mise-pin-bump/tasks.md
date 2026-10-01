# Tasks

## 1. Pins

- [x] 1.1 Set root `min_version` to `{ hard = "2026.9.13", soft = "2026.9.18" }` and verify local mise 2026.9.13 still runs root tasks and `format:check` accepts the table form
  - Evidence: local mise 2026.9.13 runs every root task with the soft warning "mise version 2026.9.18 is recommended"; the hk `format` step (`mise run format:code`, taplo included) passed at commit and pre-push.
- [x] 1.2 Set every `jdx/mise-action` `version:` input to `2026.9.18`, keeping the v4.3.0 action SHA, and verify a repository search finds no remaining `version: 2026.9.4` workflow input
  - Evidence: 29 inputs changed (bundle-build 2, ci 6, native-tests 3, quality 8, release 10); `grep -rn 2026.9.4 .github` finds nothing; `jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0` unchanged.
- [x] 1.3 Move the delivery package's workflow mise-version check, fixtures, test expectations and mise CLI citations to 2026.9.18, and verify the delivery `repo_validation` and `release_workflow` tests pass locally
  - Evidence: `published_windows.rs` now compares the first token of `mise --version` with `WORKFLOW_MISE_VERSION` ("2026.9.18"), so 2026.9.180 no longer matches. `mise run //packages/kuru-delivery:test` exit 0 (21 test binaries, 352 passed, 0 failed, 1 ignored needing a published-release candidate). `tests/support/mise_acceptance.rs` is compiled only by `apps/kuru-tui/tests/windows_mise.rs` and the published-Windows check runs only on Windows CI; both compare the live mise version and are CI evidence, not run here.

## 2. Lock drift and lockfiles

- [x] 2.1 Install the docs app's Node/npm pins with `mise install --locked node npm`, and verify on an isolated copy that the unlocked command rewrites the root `mise.lock` while the locked command leaves every lockfile unchanged
  - Evidence: isolated copy (separate mise data/cache/state/config): unlocked `mise install node npm` from `apps/kuru-docs` gave root `mise.lock` +42/-1 under 2026.9.13, again on a repeat with the tools installed; `--locked` left every lock unchanged under 2026.9.13 and 2026.9.18. In this worktree with local 2026.9.13 and no `MISE_LOCKED`, `mise run docs:check` with the unlocked task produced the same +42/-1 diff (one `specifiers` line removed), and with `--locked` produced none.
- [x] 2.2 Download mise 2026.9.18 for macos-arm64, verify it against the release `SHASUMS256.txt`, refresh the root, docs, delivery and memory `mise.lock` files with it using the documented commands (no `--upgrade`), and verify the resulting diff and that `check:repo` passes with that binary and with the repository's configured mise
  - Evidence: SHA-256 `484c135bd4329975d608d3f77e26c2ece5d2f5590f18ca71f44440294f8cfa6f` matches `SHASUMS256.txt`; it reports `2026.9.18 macos-arm64 (2026-09-30)`. With isolated mise directories, `MISE_CEILING_PATHS` at the worktrees directory, an authenticated GitHub API and `MISE_LOCKED=1`, the five-platform root, docs and delivery refreshes and the memory linux-x64 refresh exited 0 with zero diff on all four locks (lockfile_version stays 1). `check:repo` passed under 2026.9.18 with `MISE_LOCKED=1` and under local 2026.9.13 without it; neither changed a lock.

## 3. Documentation and verification

- [x] 3.1 Update the dependency audit, development and installation docs for the new pin and the corrected drift cause, and verify no stale 2026.9.4 pin references remain outside historical records
  - Evidence: the remaining `2026.9.4` mentions are the measured behavior comparisons in `docs/development.md` and archived change records.
- [x] 3.2 Run `lint:tooling`, `cospec validate --all --strict` and the hk pre-push hook without `MISE_LOCKED`, and verify each passes and `git status` shows no lockfile change afterward
  - Evidence: `mise run lint:tooling` exit 0 ("Repository metadata invariants passed"); `cospec validate --all --strict` 0 errors, 0 warnings; `hk run pre-push` exit 0 in 120s (cospec-managed, cospec, format, tooling, docs, typecheck, lint) with no `MISE_LOCKED` in the environment, and `git status --short` was empty afterward. Pre-commit hooks for each commit also ran without it.
