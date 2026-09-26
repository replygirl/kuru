## Why

The archived `old-updater-previous-release-acceptance` change could not exercise a previous release that ships shell support. `archive::verified_release` then also requires that release's paired Windows shell-support envelope, which `previous_windows_release` never downloaded, so the release after the first support-bearing one would fail before any assertion and block `publish`. Its fixture also installed only the old executable, unlike a real previous installation, so support-tree continuity across an update was unproven. Its task 2.1 and evidence claimed the support-aware branch was covered; this change records that correction.

## What Changes

- `packages/kuru-delivery/src/published_windows.rs`: when the previous release's `SHA256SUMS` or listing names its Windows shell-support envelope, download it with the same bounded HTTPS client and verify it against that `SHA256SUMS` and GitHub's asset digest before returning it; unit tests cover envelope verification and presence detection.
- `packages/kuru-delivery/tests/support/mise_acceptance.rs`: write the verified envelope beside the previous core; for a support-aware previous release, install its versioned support tree before updating, as the Windows installer does, and assert that tree survives unchanged beside the candidate's exact new snapshot.
- `docs/release.md`: describe the envelope verification and continuity check.

## Impact

Only the ignored, `KURU_STAGED_WINDOWS_ARCHIVE`-gated staged Windows test in Release's `verify-staged-windows` job changes: one additional bounded download (at most 4 MiB) when the previous release ships support. Ordinary CI gains host unit tests only.
