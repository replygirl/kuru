# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Phase Gates

The changes below are hard or soft prerequisites, but their cospec records are
not on this branch's base (`origin/main` at `501ab92d`): three are archived or
proposed only on still-open PR branches and one does not exist yet, so listing
them above would dangle and fail validation. Task 2.1 rebases after the first
three merge and then moves them into the sections above, where
`cospec sync-blockers` checks them as archived; `apply` must not run before that.

- Hard, pending merge: `previous-release-update-ci` (PR #108, branch
  `test/previous-release-update-ci`) — `packages/kuru-delivery/src/published.rs`,
  the generic `previous_release(candidate, target, token)` resolver, its
  `listed_digest`, `publishes_asset` and `verify_manifest` helpers, and the
  shared `tests/support/previous_updater.rs` helper that the target-scoped
  "no predecessor" rule extends.
- Hard, pending merge: `runner-images` (PR #106, branch `ci/runner-images`) —
  the current runner labels, the removal of the Intel macOS CI and Release legs,
  and (commit `0ed39198`) the removal of `x86_64-apple-darwin` from
  `targets.rs` (`CATALOG: [Target; 4]` plus asserts that `macos`/`x86_64` is
  rejected), `support/install.sh`, `dolt-assets.json` and `bundle_build.rs`.
  This change rebases onto it; its phase-3 catalog edit becomes `[Target; 5]`
  and keeps #106's asserts. #106 leaves the `mise.lock` `macos-x64` entries in
  place; the lock tasks here neither rely on nor remove them.
- Hard, pending merge: `native-coverage-shards` (PR #107, branch
  `ci/native-coverage-shards`) — the five-shard `coverage::SHARDS` matrix
  (including the `memory` and `runtime` shards) that D8, D11 and verification
  1.1 name.
- `dolt-source-build-inputs` (PR6a), not yet created — manifest schema v2, the
  `aarch64-pc-windows-msvc` engine asset as inert data, built-asset notices, the
  `bundle build` command and the Linux build-twice determinism job. It is listed
  under `## Soft-blocked by` once its record exists, because a `Blocked by`
  entry would make `cospec apply` exit 2 and stop the phase-2 x64
  generalizations the brief schedules before it. It is nevertheless a **hard
  gate for phase 3 and archive**: task 3.1 requires it archived before any
  arm64 job, catalog entry or workflow edit, and task 3.8 refuses to archive
  this change while it is unarchived. This departs from lead decision 4
  ("merged before PR6b starts"); the departure is recorded in design.md Open
  Questions and needs lead confirmation before phase 2 starts. D8's workflow
  wiring goes to the PR6a owner for review before task 3.4 (lead decision 4:
  "this session reviews its workflow wiring").
- PR5 and PR4b (workflow ownership), both hard gates for stage 3: another
  session owns every file under `.github/workflows` until it sends the merge
  notices for PR5 and PR4b. PR5 removes `release.yml`'s `source-tests` and
  `verify-tests`, adds the pre-bump Ubuntu `tests` job, makes `verify-staged`
  a three-OS matrix, moves the Windows label to `windows-latest`, adds the
  per-OS install/update job running `test:previous-release-update`, and
  rewrites `native-gate` around shard, collect and install. PR4b replaces the
  Unix monolithic coverage job with the uniform five-shard `coverage:shard` job
  and one collect job per OS on every OS. D8's arm64 mirror and aggregator
  edits are based on the post-PR4b file, so phase-3 tasks that edit workflows,
  and the `release_workflow.rs` gate cases that read the live workflow text,
  wait for both notices. PR4a/PR4b/PR5 precede PR6a in the maintainer's
  sequence.
