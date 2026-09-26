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
  that literal label and drives the native gate with it. `windows-latest`
  resolves to the same Windows Server 2025 image today.
- Windows arm64 is out of scope; a separate change adds its bundle input and
  runner job.
- `mise.toml`: the Rust `mr_boxington` template reduces to the `KURU_MBX`
  check; its Intel macOS clause goes.
- `packages/kuru-delivery`: the release catalog (`src/targets.rs`) drops
  `x86_64-apple-darwin`; release assembly and the published checksum count
  already follow that list, and the candidate inventory error no longer names
  a fixed count. `support/install.sh` refuses Intel Macs (detected or
  `--target x86_64-apple-darwin`) with "Intel Macs (x86_64-apple-darwin) are
  no longer supported; v0.9.0 was the last release supporting them".
- `packages/kuru-memory`: the pinned Dolt manifest drops its
  `dolt-darwin-amd64` asset; the other four assets are unchanged, and the
  manifest validator and generated catalog take their target count from one
  supported-target table and the manifest instead of a literal five.
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

User-facing: no `x86_64-apple-darwin` Release build is produced any more, and
the release assembler, bundled-engine manifest and shell installer agree on
the four remaining targets, so the next Release dispatch assembles without an
Intel archive. Intel Mac users running the installer get an explicit refusal
naming v0.9.0 as the last supporting release. Published releases, including
their archived `x86_64-apple-darwin` assets, are untouched. The `mise.lock`
files keep their `macos-x64` provenance entries: `mise lock --platform` with
the four remaining platforms refreshes but does not prune them.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
