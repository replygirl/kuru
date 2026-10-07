# Proposal

## Why

The npm Dependabot updater targets the repository root, where no `package.json` exists, so its update job fails before checking dependencies. Point it at the docs app that owns the npm manifest and lockfile.

## What Changes

- Change `.github/dependabot.yml` so the `npm` updater uses `/apps/kuru-docs`.

## Impact

Only the GitHub Dependabot npm update job is affected. No application runtime, secrets, or required product checks change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — CI-execution topology (Dependabot update job configuration)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
