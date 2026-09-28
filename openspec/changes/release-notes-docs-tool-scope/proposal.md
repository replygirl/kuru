# Proposal

## Why

The release workflow's `notes` and `build-docs` jobs still ran `mise run` with task auto-install on, so each downloaded every missing configured tool, and `notes` ran the delivery `setup` task's bare `mise install --include-task-tools`. The workflow check exempted exactly these two jobs until this follow-up scoped them; the maintainer approved both changes.

## What Changes

- `.github/workflows/release.yml`: set `MISE_TASK_RUN_AUTO_INSTALL: "false"` at workflow level and drop the eight identical job-level copies; `build-docs` gains quality.yml's named `//apps/kuru-docs:setup:tools` step; `notes` installs `//packages/kuru-delivery:setup:test-tools` (Cocogitto and Communiqué by name) instead of `setup`. No other release logic changes.
- `packages/kuru-delivery/src/repo/workflows.rs`: remove the exemption, so release.yml meets the same rule as every workflow. This is maintainer tooling behind the unshipped `tooling` feature, as in `ci-exec-auto-install`.
- `packages/kuru-delivery/tests/repo_validation.rs`: the exemption tests become a test that release.yml has none.
- `packages/kuru-delivery/tests/release_workflow.rs`: a permanent PR check derives the tools each of the two jobs runs from its steps and the mise task graph, and requires an earlier named install of each.
- `docs/development.md`, `docs/release.md`: describe the rule without the exemption, the derivation, and what a rerun can and cannot recover.

## Impact

Release jobs `notes` and `build-docs` install only named tools. Every other release job's effective environment is unchanged; `deploy-docs` gains the variable but runs no mise. No secrets, permissions, required checks or action pins change. The `lint:tooling` repository check and the delivery test suite enforce the result on every PR.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
