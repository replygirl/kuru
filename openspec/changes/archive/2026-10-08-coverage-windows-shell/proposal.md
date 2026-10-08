# Proposal

## Why

Native Windows CI invoked a PowerShell coverage body through mise's default cmd
interpreter and failed before running tests. Other Windows task bodies with
PowerShell syntax need the same explicit interpreter and failure propagation.

## What Changes

- Bind the platform, archive, and delivery coverage launchers and delivery
  advisory tasks to the interpreter their Windows bodies require.
- Remove inherited PSModulePath at owned stock PowerShell launches and preserve
  native command failures, including the coverage threshold failure.
- Extend the delivery launcher regression tests to exercise the selected shell
  and failure propagation without running coverage or refreshing advisories.

## Impact

Package-owned mise tasks and their existing delivery integration tests change.
Native CI runs the corrected launchers. Tool versions, lockfiles, coverage
minimums, application behavior, required checks, and secrets remain unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
