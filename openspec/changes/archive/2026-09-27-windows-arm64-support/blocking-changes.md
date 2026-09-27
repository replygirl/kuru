# Dependencies

## Blocked by

- [x] `previous-release-update-ci` — `packages/kuru-delivery/src/published.rs`, the generic `previous_release(candidate, target, token)` resolver, its `listed_digest`, `publishes_asset` and `verify_manifest` helpers, and the shared `tests/support/previous_updater.rs` helper that the target-scoped "no predecessor" rule extends (PR #108, merge commit `7f6c3f47`) *(archived 2026-09-26)*
- [x] `runner-images` — the current runner labels, the removal of the Intel macOS CI and Release legs, and the removal of `x86_64-apple-darwin` from `targets.rs` (`CATALOG: [Target; 4]` plus asserts that `macos`/`x86_64` is rejected), `support/install.sh`, `dolt-assets.json` and `bundle_build.rs`; this change is based on it, its phase-3 catalog edit becomes `[Target; 5]` and keeps #106's asserts, and #106's remaining `mise.lock` `macos-x64` entries are neither relied on nor removed here (PR #106, merge commit `8225613d`) *(archived 2026-09-26)*
- [x] `windows-arm64-bundle-input` — PR6a: manifest schema v2 (`packages/kuru-memory/support/dolt-assets.json`, five assets, each with `"provenance"`), the pinned source-built `aarch64-pc-windows-msvc` engine asset and its three notices, the `bundle build` command and `//packages/kuru-memory:bundle:build`/`setup:build-tools` tasks, `bundle:prepare --archive` import of a built asset, and the linux-x64 `Bundle build` determinism workflow; stage 3 imports that archive on the arm64 legs and task 3.10 refuses to archive this change before it (PR #113, merge commit `83666440`) *(archived 2026-09-26)*
- [x] `native-coverage-shards` — superseded by `coverage-partitions` (#118), which removed `coverage::SHARDS`; kept as the historical stage-3 gate. It introduced the five-shard `coverage::SHARDS` matrix (including the `memory` and `runtime` shards) that D8, D11 and verification 1.1 named before PR-C; it touches only `coverage.rs`, `release_workflow.rs`, `ci.yml`, `native-tests.yml` and `docs/development.md`, none of which stage 2 edits, and gates the stage-3 workflow and shard wiring (PR #107, merge commit `a688fe82`) *(archived 2026-09-26)*
- [x] `coverage-partitions` — PR-C: hash-assigned per-test `Coverage partition`/`Behavior partition` jobs, the uninstrumented receipts mode, the `Behavior merge`/`Coverage merge` agreement job and the `coverage::PARTITIONS`/`OS_TARGETS` tables whose fail-closed allowlist carries the `windows-11-arm` uninstrumented row that task 3.4 part (b) adds (PR #118, merge commit `98b6b81a`) *(archived 2026-09-26)*
- [x] `install-single-build` — one release build per install job, shared by `mise run install` and `bundle:verify-native-build`, which verification 4.1 and group 6 cite for both Windows tuples (PR #122, merge commit `db25ad31`) *(archived 2026-09-27)*
- [x] `dolt-2-3-4` — the bundled Dolt engine bump to v2.3.4, which re-pinned the source-built `aarch64-pc-windows-msvc` asset this change imports (PR #119, merge commit `0b951c51`) *(archived 2026-09-27)*
- [x] `dolt-2-3-5` — the bundled Dolt engine bump to v2.3.5, which moved all five engine pins, including the source-built `aarch64-pc-windows-msvc` asset this change imports; this branch's tests derive from the committed pin, so the rebase needed no test edit beyond resolving one literal in `published_windows.rs` to the derived form (PR #123, merge commit `78bbe35a`) *(archived 2026-09-27)*

## Soft-blocked by

None.

## Phase Gates

Stage 3 is based on `origin/main` at `78bbe35a`, which contains #106, #107, #108,
#110 (PR5), #111 (PR4b), #113 (PR6a), #118 (PR-C), #119, #122 and #123; every
change listed above is archived on that base. (Stage 3 began on `83666440`.)

- PR5 and PR4b (workflow ownership): the workflow owner sent both merge notices,
  so stage 3 may edit `.github/workflows` for the arm64 legs, the step-zero
  probe job and what task 3.4 names, and nothing else. The owner's follow-on
  per-test partition change (PR-C, `coverage-partitions`) merged first as #118;
  the arm64 wiring is built on it, parameterized by OS without restructuring
  shared jobs.
- PR6a merged before this change's PR opened, so draft PR #115 is a plain PR
  against main with no stack link (task 2.12).
