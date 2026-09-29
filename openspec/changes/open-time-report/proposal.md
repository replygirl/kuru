# Proposal

## Why

Phase 1 of the startup regression needs Windows and Linux open times for the release binary a user runs; no record has any, and nothing times the owner's open from inside a release build. This change adds a report-only measurement, from outside the binary, inside `ci.yml`, shaped as the first form of the open-time budget check that phase 2 will turn into a gate.

## What Changes

- `packages/kuru-delivery/src/open_time.rs` and `src/open_time/`: an `open-time` subcommand of the maintainer `kuru-delivery` tool, behind the unshipped `tooling` feature. It drives a release `kuru` in isolated scratch roots through three cases (first launch ever, first open of an existing project with no live owner, warm reopen inside the owner's idle window), timestamps its stderr progress lines, and observes processes (sysinfo) and files (stat only) under the scratch root. It writes one `kuru.open-time.v1` JSON record per run and a summary table. It never compares a time with a budget.
- `packages/kuru-delivery/tests/fixtures/delivery.rs`, `tests/open_time.rs`: a live-subprocess fixture that stands in for `kuru` and drives the harness end to end.
- `packages/kuru-delivery/mise.toml`: `measure:open-time` task.
- `Cargo.toml`, `packages/kuru-delivery/Cargo.toml`, `Cargo.lock`: `sysinfo =0.39.6` (feature `system` only), optional under `tooling`. The shipping `kuru` dependency tree is unchanged on every target.
- `.github/workflows/native-tests.yml`: the install job publishes its installed release binary as an artifact (step-level `continue-on-error`).
- `.github/workflows/ci.yml`: an `open-time` job over ubuntu-latest, macos-latest, windows-latest and windows-11-arm that downloads that binary, runs the task and uploads the records. Job-level `continue-on-error`; not in `ci-gate.needs`; no `workflow_dispatch`, no new workflow file.
- `docs/development.md`: what the job measures, that it is report-only, how to read it, and the phase 2 step that would make it a gate.

## Impact

One new non-gating job per OS in `ci.yml`, which starts after `native-tests` finishes. One extra upload step in each install job. No product code, no product dependency, no secrets, no permissions, no required checks and no action pins change. The job builds only `kuru-delivery`, which does not depend on `kuru-memory`, so it needs no engine bundle input.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
