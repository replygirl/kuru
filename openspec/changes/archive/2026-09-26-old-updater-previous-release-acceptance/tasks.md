## 1. Resolve and authenticate the previous release

- [x] 1.1 Add `published_windows::previous_windows_release` and its pure selection/verification helpers in `packages/kuru-delivery/src/published_windows.rs`, and verify with new unit tests in that file's `mod tests` covering selection (latest older stable release, candidate already published, drafts/prereleases ignored, nothing older, malformed stable tag, newer-than-candidate) and verification (exact immutable URLs, SHA256SUMS mismatch, API digest mismatch or absence).

## 2. Exercise the resolved previous updater in staged acceptance

- [x] 2.1 Replace the pinned `OLD_CLIENTS` in `packages/kuru-delivery/tests/support/mise_acceptance.rs` with the single resolved previous release, keeping exact replacement, version, regenerated-support, private-data and cleanup assertions, and branching the managed-support assertion on whether the previous core declares shell support; verify by `rustfmt --check` on the file and review, since the file compiles only on Windows.

## 3. Document the contract

- [x] 3.1 Rewrite the staged Windows acceptance step in `docs/release.md` to the previous-release contract, and verify no documentation still describes v0.4.2/v0.9.0 staged updater acceptance.

## Observed checks and release obligation

- 1.1: `cargo test -p kuru-delivery --features tooling --locked --lib published_windows` passed 9/9 on macOS, including the new `previous_release_is_the_greatest_older_stable_release` and `previous_release_assets_are_authenticated_before_use`. A scratch binary outside the repository called the real `previous_windows_release` against public GitHub: candidate 0.10.0, 0.9.1 and 1.0.0 resolved v0.9.0 (SHA256SUMS `21703d2e…17d7`, Windows ZIP `7bf971ea…49c5`, 51,296,705 bytes, identical to the digests formerly pinned in source); candidate 0.9.0 (already published) resolved v0.8.0 (ZIP `f216822d…4d97`); candidate 0.1.0 failed with "published release v0.9.0 is newer than candidate v0.1.0".
- 2.1: `rustfmt --edition 2024 --check` on the helper and `cargo fmt --all -- --check` passed. The helper is compiled only through `apps/kuru-tui/tests/windows_mise.rs` (`#![cfg(windows)]`), so no macOS build type-checks it; its new calls (`published_windows::previous_windows_release`, `PreviousWindowsRelease`, `shell_support::read_generated`, `Files` equality) were reviewed against their library signatures. The support-aware branch expects `<install>/share/kuru/<candidate>/<target>`, the path `update::replace_running` stages through `shell_support::install_versioned`.
- `mise run //packages/kuru-delivery:lint` (Clippy, all targets and features, `-D warnings`) passed.
- Not run here: the staged test itself. It remains `#[ignore]` and gated on `KURU_STAGED_WINDOWS_ARCHIVE`, and runs only in Release's `verify-staged-windows` job, whose pass is required before `publish`. Its first native execution is the next release. Today it resolves v0.9.0, an executable-only updater; the support-aware branch first executes the release after the first published release that ships shell support.
- The durable `repository-delivery` scenario naming v0.4.1/v0.4.2 strict readers belongs to the shell-support contract and the retained bootstrap fixtures, not this acceptance test; this test-only change leaves it unchanged.
