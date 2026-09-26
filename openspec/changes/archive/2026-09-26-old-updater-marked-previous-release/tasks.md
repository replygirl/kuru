## 1. Download and authenticate the previous support envelope

- [x] 1.1 Extend `previous_windows_release` in `packages/kuru-delivery/src/published_windows.rs` to fetch and verify the previous release's Windows shell-support envelope whenever its `SHA256SUMS` or listing names one, and verify with unit tests for envelope digest mismatches, listing/manifest disagreement and presence detection.

## 2. Model a real support-aware previous installation

- [x] 2.1 In `packages/kuru-delivery/tests/support/mise_acceptance.rs`, write the envelope beside the previous core, install the previous versioned support tree before updating when the previous core is marked, and assert that tree is unchanged beside the candidate's exact snapshot afterwards; verify by `rustfmt --check` and review against `install_versioned` and `update::replace_running`, since the file compiles only on Windows.

## 3. Document and correct the record

- [x] 3.1 Update `docs/release.md` for the envelope and continuity check, and record here that archived task 2.1 of `old-updater-previous-release-acceptance` overclaimed support-aware coverage.

## Correction to the archived record

Archived `2026-09-26-old-updater-previous-release-acceptance` task 2.1 and its evidence said the managed-support assertion branched correctly for a support-aware previous release. That overclaimed. For a marked previous core, `archive::verified_release` → `shell_support::verified` also reads the previous release's `kuru-<v>-x86_64-pc-windows-msvc-shell-support.zip` from the local release base, which `previous_windows_release` never downloaded. The helper would therefore have failed before any of its assertions, and blocked `publish` for the release after the first support-bearing one. The fixture also installed only the old executable, so it could not show that an existing support tree survives an update. Rows 1.1 and 2.1 above fix both; the archived record stays unedited.

## Observed checks and release obligation

- 1.1: `cargo test -p kuru-delivery --features tooling --locked --lib published_windows` passed 10/10, including new `previous_support_envelope_is_detected_and_authenticated`: detection from either the listing or `SHA256SUMS` (text or `*` binary form), absent from both for an unmarked release, listing/manifest disagreement rejected, altered envelope, substituted manifest digest and mismatched GitHub digest rejected. The whole library suite passed 60/60 with `RUST_TEST_THREADS=2`; at default parallelism unrelated `bundle::` loopback fixtures flake (`WouldBlock`), and the parent commit a7122f4c fails the same way.
- 1.1 live: the scratch caller outside the repository resolved v0.9.0 for candidate 0.10.0 and v0.8.0 for candidate 0.9.0 with `support=None`, the unchanged digests, and still refused candidate 0.1.0. No published release ships a Windows support envelope yet, so the envelope download path has only unit evidence.
- 2.1: `rustfmt --edition 2024 --check` on the helper and `cargo fmt --all -- --check` passed. The helper still compiles only on Windows; its new calls (`shell_support::archive_name`, `install_versioned`, `read_generated`, `Files` equality, `PreviousWindowsRelease::support`) were reviewed against their signatures. The previous tree is installed with `install_versioned`, which produces the same `share/kuru/<version>/<target>` layout as `install.ps1`'s `PublishSupport` and `archive::install`; the Windows updater's `replace_running` only adds the candidate's snapshot beside it. The test checks both trees by content rather than comparing returned paths.
- `mise run //packages/kuru-delivery:lint` (Clippy, all targets and features, `-D warnings`) passed.
- Not run: the staged test itself (ignored, `KURU_STAGED_WINDOWS_ARCHIVE`-gated, Release `verify-staged-windows` only). Its first run resolves executable-only v0.9.0; the support-aware branch, including envelope download and tree continuity, first executes in the release after the first published release that ships shell support.
