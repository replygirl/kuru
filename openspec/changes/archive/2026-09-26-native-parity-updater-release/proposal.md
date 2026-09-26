## Why

Ordinary CI proves installation and the installed offline runtime through a
Windows-only job and steps embedded in the Unix coverage job, and never runs the
previous published release's updater against the tree under test; the release
reruns full coverage after the version bump and accepts the staged candidate on
Windows only. One per-OS install/update job and staged acceptance on every
supported platform give the three platforms the same installation and update
floor while trimming duplicated release verification. No product behavior
changes.

## What Changes

- `.github/workflows/native-tests.yml`: one `install` job per OS (source install
  smoke, installed offline runtime, previous-release updater acceptance through
  `//packages/kuru-delivery:test:previous-release-update`, with Windows-only
  offline-input and DLL-import steps behind `runner.os`) replaces
  `windows-install` and the install steps of the Unix `coverage` job;
  `native-gate` requires it on every OS; OS step conditionals use `runner.os`.
- `.github/workflows/ci.yml`, `native-tests.yml`, `release.yml`: runner label
  `windows-2025` becomes `windows-latest`.
- `.github/workflows/release.yml`: `source-tests` and `verify-tests` are removed;
  a pre-bump `tests` job runs the ordinary `mise run test` on Ubuntu with the
  native Secret Service session; `verify-staged-windows` becomes the three-OS
  `verify-staged` matrix (Windows keeps `verify:staged-windows`; Linux x86_64
  and macOS arm64 verify `SHA256SUMS`, extract the staged archive and run
  `test:embedded-runtime` and `test:previous-release-update` against its exact
  executable); `deploy-docs` and `publish` need `verify-staged`.
- `packages/kuru-delivery/tests/release_workflow.rs`: workflow-text assertions
  for the new jobs, needs, labels and the per-OS updater step.
- `AGENTS.md`, `docs/release.md`, `docs/development.md`, `docs/verification.md`:
  staged acceptance on every supported platform, the new CI and release graph,
  and the interim Unix staged leg's limitation (it does not yet prove the mise
  installation route).
- Not included: defining `test:previous-release-update` (owned by the delivery
  change in PR #108), Unix `verify:staged` mise-route fixtures, Unix
  post-publication verification, sharding Unix coverage, build-matrix changes
  other than the Windows label.

## Impact

Jobs: `native-tests` gains `Installation, offline runtime and update (<os>)` on
all three OSes and loses `windows-install`; the Unix `coverage` job no longer
installs. The release run loses two reusable workflow calls (pre-bump native
coverage and post-bump coverage), gains `tests` and two Unix `verify-staged`
legs. The install job and the Unix staged legs pass `GITHUB_TOKEN:
${{ github.token }}` (workflow `contents: read`) only to the updater acceptance
step, which needs outbound HTTPS to GitHub. After #108, the Windows staged
`verify:staged-windows` step also re-runs that acceptance and receives the same
token for its release listing only. Required checks (`ci-gate`,
`Lint PR title`) are unchanged; `native-gate` topology changes as described.
The 90% coverage gate is enforced by CI on pull requests, merge groups and
`main`, no longer re-run by the release.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
