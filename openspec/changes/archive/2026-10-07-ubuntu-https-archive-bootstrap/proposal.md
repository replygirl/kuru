# Proposal

## Why

The Ubuntu runner's mirror-list URI routes package indexes through an HTTP Azure mirror that repeatedly stalls, even when the official Ubuntu HTTPS archive serves the release metadata. The native-test and release test bootstraps should use the working official HTTPS archive while preserving the runner's signed Ubuntu source metadata and existing apt isolation and bounds.

## What Changes

- Update the Ubuntu native-test bootstrap in `.github/workflows/native-tests.yml`.
- Keep the release test bootstrap in `.github/workflows/release.yml` coherent.
- Update the existing `FIXED_APT` workflow fixture in `packages/kuru-delivery/tests/repo_validation.rs`.

## Impact

The Ubuntu native coverage and release acceptance jobs use the official HTTPS Ubuntu archive for the existing Ubuntu source list. Apt still reads only that deb822 file, with the same suites, components, signing key, retries, and time bounds; no application runtime or secrets change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
