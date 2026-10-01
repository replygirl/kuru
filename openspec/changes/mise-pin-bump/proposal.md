# Proposal

## Why

The repository pins mise 2026.9.4 while the current release is 2026.9.18
(2026-09-30), and local hk pre-push runs rewrite the root `mise.lock` (dropping
the cospec `windows-arm64` option element's `specifiers`), which fails the
`check:repo` lock invariant unless `MISE_LOCKED=1` is exported. Measured on
isolated copies, that rewrite comes from the unlocked `mise install node npm`
behind `//apps/kuru-docs:setup:tools` and is identical under 2026.9.4, 2026.9.13
and 2026.9.18, so the version bump alone does not remove it.

## What Changes

- Root `mise.toml` `min_version` becomes `{ hard = "2026.9.13", soft = "2026.9.18" }`:
  CI runs exactly 2026.9.18, while the hard floor stays installable from the
  maintainer's package manager (Homebrew ships 2026.9.15).
- Every `jdx/mise-action` `version:` input in the bundle-build, ci, native-tests,
  quality and release workflows moves to `2026.9.18`; the action SHA (v4.3.0) is
  unchanged because it does not encode the mise version.
- The delivery package's published-Windows workflow mise-version check and
  fixture, the release-workflow and mise-acceptance test expectations, the
  repository-validation fixtures and the mise CLI citations move to 2026.9.18.
  These are version constants only; no behavior changes.
- `//apps/kuru-docs:setup:tools` installs with `mise install --locked node npm`,
  matching the delivery package's locked cargo-audit install, so hook and docs
  runs no longer rewrite lockfiles.
- The root, docs, delivery and memory `mise.lock` files are refreshed with the
  checksum-verified 2026.9.18 binary using the documented commands (five
  platforms; memory linux-x64 only), without `--upgrade` so lockfile revision 1
  remains readable by the hard floor.
- Dependency, development and installation docs record the new pin and the
  corrected drift cause.

## Impact

`mise.toml`, `apps/kuru-docs/mise.toml`, the four `mise.lock` files (if the
refresh changes them), `.github/workflows/{bundle-build,ci,native-tests,quality,release}.yml`,
`packages/kuru-delivery/src/published_windows.rs`, `packages/kuru-delivery/src/repo/workflows.rs`
(comments), `packages/kuru-delivery/tests/{release_workflow.rs,repo_validation.rs,support/mise_acceptance.rs}`,
`docs/{dependencies,development,install}.md`. CI installs mise 2026.9.18 on every
platform; the bundle-build cache key may change with the root `mise.toml` bytes.
Tests that compare the live mise binary's version (`mise_acceptance`, published
Windows verification) are proven in CI, not under the maintainer's local 2026.9.13.
