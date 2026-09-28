# Proposal

## Why

Main run 36319170835 failed Windows coverage partition 3 before any test ran:
mise could not download mr-boxington 1.18.0 (GitHub HTTP 500), a tool that CI
never uses because workflows set `KURU_MBX=0`. The job's selective
`mise install --locked` succeeded. The download came from the next step's
`rustup` call: on Windows, mise's executable shim runs `mise x -- rustup`, and
mise 2026.9.4 `exec` installs every missing configured tool unless
`exec_auto_install` is off. CI must download only the tools each job selects.

## What Changes

- `.github/workflows/ci.yml`, `quality.yml`, `native-tests.yml`,
  `bundle-build.yml`, `release.yml`: set `MISE_EXEC_AUTO_INSTALL: "false"` in
  each workflow-level `env`, beside `KURU_MBX` and the existing task
  auto-install opt-out.
- `docs/development.md` (Shared build cache): record that CI installs only
  `install_args` and why exec auto-install is disabled.
- Root `mise.toml` and `mise.lock` are unchanged; local development keeps the
  pinned mr-boxington.

## Impact

Every job using `jdx/mise-action` in those workflows. No secrets or required
check names change. A step that invokes a configured tool missing from its
job's `install_args` now warns and fails on the missing command rather than
silently downloading the entire tool set. The audited jobs call only `rustup`
and `cargo` directly; everything else goes through `mise run`, whose automatic
installation was already disabled.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
