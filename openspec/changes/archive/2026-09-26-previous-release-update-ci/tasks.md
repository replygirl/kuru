## 1. Generalize the previous-release resolver

- [x] 1.1 Move the public GitHub client, selection and authentication helpers into `packages/kuru-delivery/src/published.rs`, add `previous_release(candidate, target, token)` with per-target asset names and a metadata-only optional bearer token, replace `published_windows::previous_windows_release` at its single call site, and verify with unit tests in `published.rs` for per-target naming, token filtering, header presence/absence/sensitivity, synthetic next-patch selection and the existing selection/authentication cases.

## 2. Platform-generic previous-updater acceptance

- [x] 2.1 Add `packages/kuru-delivery/tests/support/previous_updater.rs` (host target, executable name, Unix mode, Unix isolated environment, split requested/reported version, Unix stable man page for support-aware updaters) and route `mise_acceptance.rs::run_staged` through it, and verify by clippy/typecheck on the host and review of the Windows-only call site.
- [x] 2.2 Add the ignored `packages/kuru-delivery/tests/previous_release_update.rs` with its `[[test]]` entry and the `test:previous-release-update` mise task, and verify by listing the test and running the task for real against the host's previous published release.

## 3. Documentation

- [x] 3.1 Update `docs/release.md`, `docs/development.md`, `AGENTS.md`, the helper comment and the resolver doc comment to describe previous-release acceptance as a floor run natively in CI on the native-test platforms, and verify with `docs:check`-level formatting and link checks.

## 4. Verification

- [x] 4.1 Run `format:check`, `lint`, `typecheck`, `lint:tooling` and `//packages/kuru-delivery:test`, and record observed results here.

## Observed checks

All on macOS arm64 (aarch64-apple-darwin) in the branch worktree, 2026-09-26.

- 1.1: `cargo test -p kuru-delivery --features tooling --locked --lib published` passed 14/14, including the new `previous_release_assets_are_named_per_target`, `synthetic_next_patch_candidate_selects_the_published_workspace_release`, `optional_token_is_normalized_without_accepting_malformed_values` and `token_is_sent_only_on_api_metadata_requests`, plus the moved selection/authentication tests.
- 2.1/2.2: `cargo test -p kuru-delivery --features tooling --locked --test previous_release_update -- --list` listed the one ignored test. `mise run build:release` built `target/release/kuru` (`kuru 0.9.0`). `mise run //packages/kuru-delivery:test:previous-release-update` with `KURU_UPDATE_CANDIDATE_BINARY` set to that binary first failed anonymously with `HTTP status client error (403 rate limit exceeded)` on `https://api.github.com/repos/replygirl/kuru/releases` (this host's anonymous quota was exhausted); rerun with a read `GITHUB_TOKEN` it passed in 21s: candidate v0.9.1 (built as v0.9.0), previous release v0.9.0 `aarch64-apple-darwin` archive `3ec49491…3291`, shell support absent; "previous v0.9.0 aarch64-apple-darwin updater installed candidate v0.9.1: Installed Kuru 0.9.1 at …/old-installed-bin/kuru", `test result: ok. 1 passed`.
- 4.1: `mise run format:check`, `mise run lint`, `mise run typecheck`, `mise run lint:tooling`, `mise run docs:check`, `mise run cospec:managed:check` and `mise run //packages/kuru-delivery:test` (lib 73/73, all integration binaries ok) exited 0.
- Not run here: the Windows paths. `mise_acceptance.rs` and the Windows branch of `previous_updater.rs` compile only on Windows, and a macOS cross-check stops in `aws-lc-sys` without Windows headers; the call site and helper were reviewed against the Windows `command::Command` and `mise_isolation::prepare` signatures. The release-time `verify-staged-windows` run is their first native execution unless the workflow owner wires the new task on Windows first. The support-aware previous-updater branch and the Unix stable man-page check first execute once a published release ships shell support (v0.9.0 does not). Linux targets run only once CI wires the task. Coverage was not run as a standalone step; it ran inside the branch's hk pre-push hook and passed the 90% workspace line gate.
