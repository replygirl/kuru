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
  - `.github/workflows/native-tests.yml`: the per-OS `shard` job's
    `native-coverage-${{ inputs.os }}` key (combined with its existing
    `matrix.shard == 'connectors-core-platform'` condition; the `collect`
    job's own step already restores only, `save-if: false`, and needs no
    change), and `native-install-${{ inputs.os }}` (renamed from
    `native-install-windows` by main's #110, which also expanded that job to
    run and cache installs on Linux and macOS, not only Windows)
  - `.github/workflows/ci.yml`: `native-build-${{ matrix.target }}` and
    `native-platform-windows`
  - `.github/workflows/release.yml` calls the same reusable `quality.yml` and
    `native-tests.yml` workflows, so it inherits this behavior without its own
    edit.
- Replace the source-built `cargo:cargo-llvm-cov` mise pin with the prebuilt
  `aqua:taiki-e/cargo-llvm-cov` backend at the same exact version `0.9.1`, with
  checksums recorded in `mise.lock` and `packages/kuru-delivery/mise.lock`, in
  `mise.toml`, `packages/kuru-delivery/mise.toml`, and the `install_args` in
  `.github/workflows/ci.yml` and `.github/workflows/native-tests.yml`.
  `docs/dependencies.md` reflects the new tool address. The coverage tasks'
  `LLVM_COV_VERSION`/version-check strings are unchanged. (This PR was
  authored against pre-#111 main, where `packages/kuru-delivery/mise.toml`
  and `support/windows-coverage.ps1` also carried the tool address; #111
  merged first, replaced that PowerShell path with the unified
  `coverage shard`/`collect` Rust orchestrator, and deleted
  `windows-coverage.ps1`, so this change absorbed that rebase and moved the
  swap to `packages/kuru-delivery/src/coverage/orchestrate.rs`'s
  `LLVM_COV_TOOL` const instead, including its two error-message test
  assertions in `orchestrate.rs`.)
- Update `packages/kuru-delivery/tests/release_workflow.rs`'s exact-match
  assertion on the coverage shard `save-if` string to the new combined
  condition, and add `every_rust_cache_step_restricts_saves_to_main`, which
  scans `ci.yml`/`quality.yml`/`native-tests.yml` and fails if any
  `Swatinem/rust-cache` step's `with:` block lacks either the main-ref
  condition or a hard-coded `save-if: false` — the exact class of step the
  review caught omitted (`native-platform-windows`) and the class a careless
  rebase could omit again (`native-install-${{ inputs.os }}`).

## Impact

- Jobs: `quality` (lint/typecheck/tooling/docs), `native-tests` (`shard`,
  `collect`, `install`), `native-build` and native-platform primitives in
  `ci.yml`, and everywhere `release.yml` calls those reusable workflows. No
  job's required checks, names, or pass/fail semantics change — only whether
  a job's `rust-cache` step *saves* at the end, and how `cargo-llvm-cov` is
  installed.
- No secrets touched. No application code, specs, or runtime behavior changed.
- `mise.lock` and `packages/kuru-delivery/mise.lock` gain prebuilt-binary
  checksums/URLs for `cargo-llvm-cov` 0.9.1 across linux/macos/windows
  platforms in place of the old cargo-backend entries.

## Surfaces

- [ ] interactive
- [x] deploy
- [ ] integration
- [ ] agent-behavior
