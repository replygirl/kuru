# Tasks

## 1. Pins

- [ ] 1.1 Set root `min_version` to `{ hard = "2026.9.13", soft = "2026.9.18" }` and verify local mise 2026.9.13 still runs root tasks and `format:check` accepts the table form
- [ ] 1.2 Set every `jdx/mise-action` `version:` input to `2026.9.18`, keeping the v4.3.0 action SHA, and verify a repository search finds no remaining `version: 2026.9.4` workflow input
- [ ] 1.3 Move the delivery package's workflow mise-version check, fixtures, test expectations and mise CLI citations to 2026.9.18, and verify the delivery `repo_validation` and `release_workflow` tests pass locally

## 2. Lock drift and lockfiles

- [ ] 2.1 Install the docs app's Node/npm pins with `mise install --locked node npm`, and verify on an isolated copy that the unlocked command rewrites the root `mise.lock` while the locked command leaves every lockfile unchanged
- [ ] 2.2 Download mise 2026.9.18 for macos-arm64, verify it against the release `SHASUMS256.txt`, refresh the root, docs, delivery and memory `mise.lock` files with it using the documented commands (no `--upgrade`), and verify the resulting diff and that `check:repo` passes with that binary and with the repository's configured mise

## 3. Documentation and verification

- [ ] 3.1 Update the dependency audit, development and installation docs for the new pin and the corrected drift cause, and verify no stale 2026.9.4 pin references remain outside historical records
- [ ] 3.2 Run `lint:tooling`, `cospec validate --all --strict` and the hk pre-push hook without `MISE_LOCKED`, and verify each passes and `git status` shows no lockfile change afterward
