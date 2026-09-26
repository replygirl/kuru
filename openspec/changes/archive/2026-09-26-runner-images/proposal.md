## Why

CI pins aging GitHub-hosted runner labels (`macos-14` is deprecated upstream),
and the maintainer has decided to stop building, testing and publishing Intel
macOS (`x86_64-apple-darwin`). Moving to the current `-latest` aliases keeps
CI on supported images without per-image churn.

## What Changes

- `.github/workflows/ci.yml`, `native-tests.yml`, `quality.yml`,
  `release.yml`, `pr-title.yml`: `ubuntu-24.04` (x64) becomes `ubuntu-latest`
  and `macos-14` becomes `macos-latest` (macOS 26 arm64), including the
  reusable native-tests `os` default and every `inputs.os` comparison that
  selects the Ubuntu-only steps.
- Remove the `macos-15-intel` / `x86_64-apple-darwin` legs from the CI
  `native-build` matrix and the Release `build` matrix.
- `ubuntu-24.04-arm` stays: runner-images publishes no arm64 Linux `-latest`
  alias.
- `windows-2025` stays: `kuru-delivery`'s `tests/release_workflow.rs` pins
  that literal label and drives the native gate with it, and this change may
  not edit `packages/*`. `windows-latest` resolves to the same Windows Server
  2025 image today.
- Windows arm64 is out of scope; a separate change adds its bundle input and
  runner job.
- `mise.toml`: the Rust `mr_boxington` template reduces to the `KURU_MBX`
  check; its Intel macOS clause goes.
- `docs/development.md`, `docs/install.md`, `docs/dependencies.md`,
  `docs/verification.md`, `docs/release.md`,
  `README.md`, `apps/kuru-docs/guide/installation.md`: remove Intel macOS
  support and CI claims.

## Impact

CI job names change with their labels (for example `native-tests
(ubuntu-latest)`, `native-tests (macos-latest)`), and the CI `native-build`
Intel leg disappears; `ci-gate` still requires every remaining job. Coverage
artifact names derived from the OS label change accordingly; nothing downloads
them by the old name. No secrets change.

User-facing: no `x86_64-apple-darwin` Release build is produced any more. The
Release assembler in `packages/kuru-delivery` (`src/targets.rs` catalog,
`src/release.rs` archive and checksum-count checks) still requires an
`x86_64-apple-darwin` archive, so the next Release dispatch fails at
`assemble-candidate` until a follow-on `kuru-delivery`/`kuru-memory` change
drops that target. That follow-on is outside this change's ownership. This
does not block the PR; it blocks the next release.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
