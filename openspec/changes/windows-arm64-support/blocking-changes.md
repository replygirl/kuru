# Dependencies

## Blocked by

- [x] `previous-release-update-ci` — `packages/kuru-delivery/src/published.rs`, the generic `previous_release(candidate, target, token)` resolver, its `listed_digest`, `publishes_asset` and `verify_manifest` helpers, and the shared `tests/support/previous_updater.rs` helper that the target-scoped "no predecessor" rule extends (PR #108, merge commit `7f6c3f47`) *(archived 2026-09-26)*
- [x] `runner-images` — the current runner labels, the removal of the Intel macOS CI and Release legs, and the removal of `x86_64-apple-darwin` from `targets.rs` (`CATALOG: [Target; 4]` plus asserts that `macos`/`x86_64` is rejected), `support/install.sh`, `dolt-assets.json` and `bundle_build.rs`; this change is based on it, its phase-3 catalog edit becomes `[Target; 5]` and keeps #106's asserts, and #106's remaining `mise.lock` `macos-x64` entries are neither relied on nor removed here (PR #106, merge commit `8225613d`) *(archived 2026-09-26)*
- [x] `windows-arm64-bundle-input` — PR6a: manifest schema v2 (`packages/kuru-memory/support/dolt-assets.json`, five assets, each with `"provenance"`), the pinned source-built `aarch64-pc-windows-msvc` engine asset and its three notices, the `bundle build` command and `//packages/kuru-memory:bundle:build`/`setup:build-tools` tasks, `bundle:prepare --archive` import of a built asset, and the linux-x64 `Bundle build` determinism workflow; stage 3 imports that archive on the arm64 legs and task 3.10 refuses to archive this change before it (PR #113, merge commit `83666440`) *(archived 2026-09-26)*
- [x] `native-coverage-shards` — the five-shard `coverage::SHARDS` matrix (including the `memory` and `runtime` shards) that D8, D11 and verification 1.1 name; it touches only `coverage.rs`, `release_workflow.rs`, `ci.yml`, `native-tests.yml` and `docs/development.md`, none of which stage 2 edits, and gates the stage-3 workflow and shard wiring (PR #107, merge commit `a688fe82`) *(archived 2026-09-26)*

## Soft-blocked by

None.

## Phase Gates

Stage 3 is based on `origin/main` at `83666440`, which contains #106, #107, #108,
#110 (PR5), #111 (PR4b) and #113 (PR6a); every change listed above is archived
on that base.

- PR5 and PR4b (workflow ownership): the workflow owner sent both merge notices,
  so stage 3 may edit `.github/workflows` for the arm64 legs, the step-zero
  probe job and what task 3.4 names, and nothing else. The owner's follow-on
  per-test partition change (PR-C, branch `ci/coverage-partitions`) also edits
  `native-tests.yml` and `coverage.rs`; the arm64 wiring stays parameterized by
  OS without restructuring shared jobs so that either change rebases trivially
  onto the other.
- PR6a merged before this change's PR opened, so draft PR #115 is a plain PR
  against main with no stack link (task 2.12).
