## Why

The repository's GitHub Actions cache sits over its default limit (13.65 GB across
39 entries by `GET /repos/replygirl/kuru/actions/cache/usage`), because every
branch — not just `main` — saves the large `quality-*`, `native-install-*`,
`native-build-*` and coverage caches. Eviction runs oldest-first, so a warm
coverage key saved on a PR was evicted within about 90 minutes and `main`'s own
caches have been evicted too. Separately, `cargo:cargo-llvm-cov` is a source-built
mise pin, so every cold job compiles it (53 s–1m45 depending on OS) instead of
downloading a prebuilt binary.

## What Changes

- Restrict `Swatinem/rust-cache` saves to `main` only (`save-if: github.ref ==
  'refs/heads/main'`, combined with each job's existing `save-if` where one
  already exists) for the quality, install, native-build and coverage cache
  keys:
  - `.github/workflows/quality.yml`: `quality-lint`, `quality-typecheck`,
    `quality-tooling`, `quality-docs`
  - `.github/workflows/native-tests.yml`: `native-coverage`,
    `native-coverage-windows` (combined with its existing
    `matrix.shard == 'connectors-core-platform'` condition),
    `native-install-${{ inputs.os }}` (renamed from `native-install-windows`
    by main's #110, which also expanded that job to run and cache installs on
    Linux and macOS, not only Windows)
  - `.github/workflows/ci.yml`: `native-build-${{ matrix.target }}` and
    `native-platform-windows`
  - `.github/workflows/release.yml` calls the same reusable `quality.yml` and
    `native-tests.yml` workflows, so it inherits this behavior without its own
    edit.
- Replace the source-built `cargo:cargo-llvm-cov` mise pin with the prebuilt
  `aqua:taiki-e/cargo-llvm-cov` backend at the same exact version `0.9.1`, with
  checksums recorded in `mise.lock`, in `mise.toml`,
  `packages/kuru-delivery/mise.toml`,
  `packages/kuru-delivery/support/windows-coverage.ps1`, and the
  `install_args` in `.github/workflows/ci.yml` and
  `.github/workflows/native-tests.yml`. `docs/dependencies.md` reflects the new
  tool address. The coverage tasks' `LLVM_COV_VERSION`/version-check strings
  are unchanged.
- Update `packages/kuru-delivery/tests/release_workflow.rs`'s exact-match
  assertion on the Windows coverage `save-if` string to the new combined
  condition.

## Impact

- Jobs: `quality` (lint/typecheck/tooling/docs), `native-tests` (coverage,
  windows-coverage, install), `native-build` and native-platform primitives in
  `ci.yml`, and
  everywhere `release.yml` calls those reusable workflows. No job's required
  checks, names, or pass/fail semantics change — only whether a job's
  `rust-cache` step *saves* at the end, and how `cargo-llvm-cov` is installed.
- No secrets touched. No application code, specs, or runtime behavior changed.
- `mise.lock` gains prebuilt-binary checksums/URLs for `cargo-llvm-cov`
  0.9.1 across linux/macos/windows platforms in place of the old
  cargo-backend entry.
- Follow-up required for #111 (`ci/coverage-shard-orchestrator`, not yet
  merged): that branch hard-codes the `cargo:cargo-llvm-cov@0.9.1` tool
  address in `packages/kuru-delivery/src/coverage/orchestrate.rs`
  (`LLVM_COV_TOOL`) and in `native-tests.yml`'s `mise exec
  cargo:cargo-llvm-cov@0.9.1 -- ...` steps. Once this change reaches `main`,
  that branch must rebase and update those addresses to
  `aqua:taiki-e/cargo-llvm-cov@0.9.1` or it will resolve the wrong (unpinned)
  tool.

## Surfaces

- [ ] interactive
- [x] deploy
- [ ] integration
- [ ] agent-behavior
