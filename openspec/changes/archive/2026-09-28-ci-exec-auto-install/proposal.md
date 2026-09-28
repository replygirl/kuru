# Proposal

## Why

Runs 36319170835 and 36453397286 failed on fetches the jobs did not need or the
run had already verified: a Windows shim's `mise x` downloading unselected
tools, `apt-get update` refreshing a third-party runner list, and every
partition downloading the same Dolt archive. CI must fetch only what each job
uses, and a repository check must keep that class from returning.

## What Changes

- `.github/workflows/ci.yml`, `quality.yml`, `native-tests.yml`,
  `bundle-build.yml`: workflow-level `MISE_EXEC_AUTO_INSTALL: "false"` and
  `MISE_TASK_RUN_AUTO_INSTALL: "false"` (ci.yml's five job-level task settings
  move to its workflow level; no job's effective value changes).
- `.github/workflows/release.yml`: only the workflow-level
  `MISE_EXEC_AUTO_INSTALL: "false"` and the apt source restriction in the Tests
  job's fixture step. The `notes` and `build-docs` jobs still run `mise run`
  with task auto-install on, downloading tools they do not use, and `notes`
  runs `setup`'s bare `mise install --include-task-tools`. This gap is live and
  exempted by name; follow-up `release-notes-docs-tool-scope` scopes their tool
  installation, a release workflow change for the maintainer to decide.
- `.github/workflows/native-tests.yml`: the same apt restriction in its fixture
  step.
- `.github/workflows/ci.yml`, `native-tests.yml`: a `bundle-inputs` job fetches
  and verifies each pinned upstream Dolt archive once; partitions import it with
  `bundle:prepare --archive --offline` under `KURU_DOLT_BUNDLE_OFFLINE: "true"`.
- `packages/kuru-delivery/src/repo/workflows.rs` (repository tooling behind the
  `tooling` feature, run by `lint:tooling`), `Cargo.toml`, `Cargo.lock` (one
  `serde_yaml_ng` edge from the existing `=0.10.0` pin): the workflow rules.
- `packages/kuru-delivery/tests/repo_validation.rs`,
  `tests/release_workflow.rs`: rule and structure tests.
- `docs/development.md`: the class rule, the live exemption and its follow-up.

## Impact

Every job using mise in those workflows, the native partitions (which now wait
for `Verified bundle inputs`, a new job `ci-gate` requires) and the two apt
fixture steps. No secrets or existing required check names change. Release
jobs run the same steps: exec auto-install being off stops their Windows shims
installing unselected tools, and the Tests job's apt step reads only the Ubuntu
archive list.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
