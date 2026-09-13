## Why

The native Windows platform coverage task writes its LCOV report before enforcing
the 90% line threshold. A threshold failure currently skips the following upload,
so reviewers cannot inspect the uncovered lines that caused the required failure.

## What Changes

- Update `.github/workflows/ci.yml` so the native-platform LCOV upload runs after
  a failed coverage threshold when the exact report file exists and the job was
  not cancelled, while a successful run still fails on a missing report.
- Retain the existing 90% threshold, lock check, and artifact missing-file policy.

## Impact

- Affects only the Windows native-platform CI job and its diagnostic artifact.
- Does not change application source, test selection, coverage requirements,
  secrets, or required-check outcomes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
