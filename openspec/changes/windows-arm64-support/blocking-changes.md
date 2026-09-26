# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Phase Gates

The changes below are hard or soft prerequisites, but their cospec records are
not on this branch's base (`origin/main` at `501ab92d`): two are archived only
on their still-open PR branches and one does not exist yet, so listing them
above would dangle and fail validation. Task 2.1 rebases after the first two
merge and then moves them into the sections above, where `cospec sync-blockers`
checks them as archived; `apply` must not run before that.

- Hard, pending merge: `previous-release-update-ci` (PR #108, branch
  `test/previous-release-update-ci`) — `packages/kuru-delivery/src/published.rs`,
  the generic `previous_release(candidate, target, token)` resolver and the
  shared `tests/support/previous_updater.rs` helper that the target-scoped
  "no predecessor" rule extends.
- Hard, pending merge: `runner-images` (PR #106, branch `ci/runner-images`) —
  the current runner labels and the removal of the Intel macOS CI and Release
  legs that the `windows-11-arm` mirror is designed against.
- Soft, not yet created: `dolt-source-build-inputs` (PR6a) — manifest schema
  v2, the `aarch64-pc-windows-msvc` engine asset as inert data, the
  `bundle build` command and the Linux build-twice determinism job. Without it
  every phase-2 task still builds and tests on x64, but no arm64 job can prepare
  its engine and phase 3 cannot start.
- PR5 (workflow ownership): another session owns every file under
  `.github/workflows` until it sends its PR5 merge notice. Phase-3 tasks that
  edit workflows wait for that notice. PR4a/PR4b/PR5 precede PR6a in the
  maintainer's sequence.
- The Intel macOS catalog removal that PR #106 defers to a follow-on
  (`targets.rs`, `dolt-assets.json`, `install.sh`, `macos-x64` lock entries) is
  a separate change. This change adds the arm64 catalog entry without asserting
  a final catalog length and rebases onto whichever order lands.
