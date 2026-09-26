## Why

Kuru is v0 with no users, so the maintainer's upgrade contract requires compatibility only from the immediately previous published release. The staged Release acceptance should therefore exercise exactly that release's own updater against every candidate, resolved at run time, instead of pinning v0.4.2 and v0.9.0 digests that go stale on every release.

## What Changes

- `packages/kuru-delivery/src/published_windows.rs`: add `previous_windows_release`, which lists published GitHub releases, selects the greatest stable `vX.Y.Z` release other than the candidate (failing closed if it is not older, or if a stable release tag is not `vX.Y.Z`), downloads its `SHA256SUMS` and Windows ZIP over HTTPS from the immutable version path, and verifies the ZIP against that `SHA256SUMS` and both files against GitHub's asset digests before returning them. Pure selection and verification functions gain unit tests.
- `packages/kuru-delivery/tests/support/mise_acceptance.rs`: drop the pinned v0.4.2/v0.9.0 `OLD_CLIENTS`; run the single resolved previous release's updater against the staged candidate with the existing assertions (exact replacement, upgraded version, regenerated support matching the staged sidecar, no private data, cleanup). An executable-only previous release must leave no managed support; a support-aware previous release must install the staged candidate's exact versioned support snapshot.
- `docs/release.md`: state the previous-release contract.

## Impact

Only the ignored, `KURU_STAGED_WINDOWS_ARCHIVE`-gated staged Windows test run by Release's `verify-staged-windows` job changes behavior: one unauthenticated GitHub API request, one old-release download and one old `kuru update` run instead of two pinned ones. Ordinary PR/main CI gains only the new host unit tests.
