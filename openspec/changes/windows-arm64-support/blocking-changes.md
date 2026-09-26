# Dependencies

## Blocked by

- [x] `previous-release-update-ci` — `packages/kuru-delivery/src/published.rs`, the generic `previous_release(candidate, target, token)` resolver, its `listed_digest`, `publishes_asset` and `verify_manifest` helpers, and the shared `tests/support/previous_updater.rs` helper that the target-scoped "no predecessor" rule extends (PR #108, merge commit `7f6c3f47`) *(archived 2026-09-26)*
- [x] `runner-images` — the current runner labels, the removal of the Intel macOS CI and Release legs, and the removal of `x86_64-apple-darwin` from `targets.rs` (`CATALOG: [Target; 4]` plus asserts that `macos`/`x86_64` is rejected), `support/install.sh`, `dolt-assets.json` and `bundle_build.rs`; this change is based on it, its phase-3 catalog edit becomes `[Target; 5]` and keeps #106's asserts, and #106's remaining `mise.lock` `macos-x64` entries are neither relied on nor removed here (PR #106, merge commit `8225613d`) *(archived 2026-09-26)*

## Soft-blocked by

None.

## Phase Gates

Stage 2 is based on `origin/main` at `8225613d`, which contains #106 and #108
(listed above as archived hard blockers). The changes below gate stage 3
(task 3.1) and archive, not stage 2; they are kept here rather than above
because their records are either already archived without gating stage 2's
files or do not exist yet on this base.

- Hard, stage 3: `native-coverage-shards` (PR #107, merged at `a688fe82`,
  record archived on this base) — the five-shard `coverage::SHARDS` matrix
  (including the `memory` and `runtime` shards) that D8, D11 and verification
  1.1 name. It touches only `coverage.rs`, `release_workflow.rs`, `ci.yml`,
  `native-tests.yml` and `docs/development.md`, none of which stage 2 edits, so
  it gates the stage-3 workflow and shard wiring beside PR5 and PR4b, not
  stage 2.
- `dolt-source-build-inputs` (PR6a, open as #113 on branch
  `feat/windows-arm64-bundle-input`; its record is not on this base yet) —
  manifest schema v2, the `aarch64-pc-windows-msvc` engine asset as inert data,
  built-asset notices, the `bundle build` command and the Linux build-twice
  determinism job. It is listed under `## Soft-blocked by` once its record
  exists on this branch's base, because a `Blocked by` entry would make
  `cospec apply` exit 2 and stop the stage-2 x64 generalizations. It is
  nevertheless a **hard gate for stage 3 and archive**: task 3.1 requires it
  archived before any arm64 job, catalog entry or workflow edit, and task 3.10
  refuses to archive this change while it is unarchived. The departure from
  lead decision 4 ("merged before PR6b starts") is settled by the lead's
  stack-order ruling of 2026-09-26 (design.md Open Question 3): stage-1 and
  stage-2 work proceeds on a local branch off main and rebases onto PR6a's
  branch once it opens. D8's workflow wiring goes to the PR6a owner for review
  before task 3.4 (lead decision 4: "this session reviews its workflow
  wiring").
- PR5 and PR4b (workflow ownership), both hard gates for stage 3: another
  session owns every file under `.github/workflows` until it sends the merge
  notices for PR5 and PR4b. PR5 removes `release.yml`'s `source-tests` and
  `verify-tests`, adds the pre-bump Ubuntu `tests` job, makes `verify-staged`
  a three-OS matrix, moves the Windows label to `windows-latest`, adds the
  per-OS install/update job running `test:previous-release-update`, and
  rewrites `native-gate` around shard, collect and install. PR4b replaces the
  Unix monolithic coverage job with the uniform five-shard `coverage:shard` job
  and one collect job per OS on every OS. D8's arm64 mirror and aggregator
  edits are based on the post-PR4b file, so stage-3 tasks that edit workflows,
  and the `release_workflow.rs` gate cases that read the live workflow text,
  wait for both notices. PR4a/PR4b/PR5 precede PR6a in the maintainer's
  sequence.
