## Why

Updating Kuru must always be possible and always succeed; migrations exist for that reason. The acceptance that the immediately previous published release's own updater installs a new candidate is a floor under that policy, yet today it runs only on Windows inside the Release workflow, after a candidate already exists. This change makes the same acceptance a platform-generic, package-owned task that ordinary PR/main CI can run natively on its native-test platforms (Linux x86-64, macOS Apple Silicon and Windows x86-64), with the release-time Windows run kept as a sanity re-run.

## What Changes

- `packages/kuru-delivery/src/published.rs` (new, `tooling`): the public GitHub client, stable-release selection and asset authentication move here from `published_windows.rs`, generalized to `previous_release(candidate, target, token)` with per-target archive and shell-support names (`.tar.gz` on Unix targets, `.zip` on Windows). An optional `GITHUB_TOKEN` is sent as `Authorization: Bearer` only on the api.github.com metadata listing, never on asset downloads and never to any Kuru or mise child. Unit tests cover per-target naming, token filtering and header presence/absence/sensitivity, and the existing selection and authentication cases.
- `packages/kuru-delivery/src/published_windows.rs`: imports the shared client. `previous_windows_release` is replaced by `published::previous_release` with the Windows target at its single call site. The post-publication verifier sends no token and is otherwise unchanged.
- `packages/kuru-delivery/tests/support/previous_updater.rs` (new): the previous-updater acceptance helper, generic over host target, executable name, Unix mode bits and a Unix isolated environment (HOME, XDG roots, TMPDIR, PATH=/usr/bin:/bin). It asserts the requested release version separately from the version the upgraded binary reports; exact replacement bytes remain the discriminating check. On Unix a support-aware previous updater must also publish the staged stable man page.
- `packages/kuru-delivery/tests/support/mise_acceptance.rs`: the staged Windows check calls the shared helper with unchanged behaviour.
- `packages/kuru-delivery/tests/previous_release_update.rs` (new, ignored, `tooling`): packages `KURU_UPDATE_CANDIDATE_BINARY` and its generated shell support under a synthetic next patch version, resolves the host target's previous published release, and runs the shared helper.
- `packages/kuru-delivery/mise.toml`: `test:previous-release-update` runs that test single-threaded. Workflow wiring is owned separately and is not part of this change.
- Docs: `docs/release.md`, `docs/development.md` and `AGENTS.md` describe previous-release acceptance as a floor running natively in CI on the native-test platforms, with the release job as a sanity re-run.

The behavior is already specified by the release and update requirements; this change adds tests, a task and documentation. No product behaviour changes.

## Impact

Ordinary coverage gains only the new resolver unit tests; the ignored acceptance adds none. Each CI invocation of the new task makes one GitHub API request, downloads one previous release archive (about 50 MB) and runs one old `kuru update`. A branch whose workspace version is older than the latest published tag fails release selection by design and must be rebased.
