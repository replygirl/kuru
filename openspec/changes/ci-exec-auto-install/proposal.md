# Proposal

## Why

Main run 36319170835 failed Windows coverage partition 3 before any test ran:
mise could not download mr-boxington 1.18.0 (GitHub HTTP 500), a tool that CI
never uses because workflows set `KURU_MBX=0`. The job's selective
`mise install --locked` succeeded. The download came from the next step's
`rustup` call: on Windows, mise's executable shim runs `mise x -- rustup`, and
mise 2026.9.4 `exec` installs every missing configured tool unless
`exec_auto_install` is off. CI must download only the tools each job selects.
Run 36453397286 then failed twice more on the same class, a job failing on a
fetch it does not need or that the run has already satisfied: Ubuntu coverage
partition 7's `apt-get update` got 403 from `packages.microsoft.com`, a runner
image list the step never uses, and Windows on Arm partition 2 got HTTP 500
downloading the Dolt Windows x64 archive that every partition fetched itself.
The class is prevented by a repository check in CI, not by instance fixes alone.

## What Changes

- `.github/workflows/ci.yml`, `quality.yml`, `native-tests.yml`,
  `bundle-build.yml`, `release.yml`: set `MISE_EXEC_AUTO_INSTALL: "false"` in
  each workflow-level `env`, beside `KURU_MBX` and the existing task
  auto-install opt-out.
- `docs/development.md` (Shared build cache): record that CI installs only
  `install_args` and why exec auto-install is disabled.
- Root `mise.toml` and `mise.lock` are unchanged; local development keeps the
  pinned mr-boxington.
- `packages/kuru-delivery/src/repo/workflows.rs` (repository tooling behind
  the `tooling` feature, run by `check:repo` in `lint:tooling`; no application
  source): reject a mise workflow without workflow-level
  `MISE_EXEC_AUTO_INSTALL: "false"`, any job, step or script override of it, a
  `jdx/mise-action` step without `install_args`, a `mise install` without tool
  arguments, an apt fetch that does not name its source list and disable the
  parts directory, and a `partition` matrix job without job-level
  `KURU_DOLT_BUNDLE_OFFLINE: "true"` or with a step override. Tests in
  `tests/repo_validation.rs` reject each incident's pre-fix text and accept the
  fixed workflows. `serde_yaml_ng` (workspace pin `=0.10.0`, already locked)
  becomes an optional `tooling` dependency; Cargo.lock gains only that edge.
- `native-tests.yml`, `release.yml` (Tests job fixture step only; no release
  planning, build, notes or publication logic): the secret-store fixture step
  reads only the image's Ubuntu archive list
  (`-o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources
  -o Dir::Etc::sourceparts=/dev/null`) for both `update` and `install`.
- `ci.yml`: a new `bundle-inputs` job prepares each pinned upstream Dolt archive
  once through `bundle:prepare` and publishes one artifact per partition label.
  `native-tests` (new required input `bundle-inputs`) and `native-memory`
  partitions download their label's set, import it with `bundle:prepare
  --archive --offline` into their own private bundle directory and run with
  `KURU_DOLT_BUNDLE_OFFLINE: "true"`. `ci-gate` also requires the new job.
- `tests/release_workflow.rs`: structural tests follow the new jobs and steps,
  and each label's set is checked against `coverage::OS_TARGETS` and the
  manifest's provenance.
- `docs/development.md` (Commands): the class rule and what the check covers.

## Impact

Every job using `jdx/mise-action` in those workflows. No secrets or required
check names change. A step that invokes a configured tool missing from its
job's `install_args` now warns and fails on the missing command rather than
silently downloading the entire tool set. The audited jobs call only `rustup`
and `cargo` directly; everything else goes through `mise run`, whose automatic
installation was already disabled.

CI gains the `Verified bundle inputs` job, which every native partition now
waits for; `ci-gate` requires it. Its one download per archive per run replaces
one per partition. The installation, native build and release jobs still prepare
their own single engine download. No secrets change; the new artifacts carry
only pinned public archives, and each consumer verifies them again.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
