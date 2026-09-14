## Why

Windows native coverage takes about 34 minutes inside a roughly 41–47 minute
job, while Linux and macOS coverage take about six minutes. Independent package
shards can shorten that critical path if one fail-closed workspace report still
enforces the existing Windows coverage and test inventory.

## What Changes

- Add package-owned delivery tasks and a small CI helper that compile the full
  instrumented Cargo graph, dispatch its standard test targets through Cargo's
  native runner, and validate exact shard ledgers, receipts and raw profiles.
- Split Windows native coverage into four package shards and one aggregate
  workspace report while leaving Linux and macOS coverage unchanged.
- Run the existing Windows source-install and offline installed-runtime checks
  in a separate exact-source job.
- Validate the workflow topology and document the measured scheduling change.

## Impact

This changes `.github/workflows/native-tests.yml`, delivery-owned CI tooling and
tests, and `docs/development.md`. The existing reusable workflow entrypoint,
secrets, native gate, per-OS 90% workspace line threshold and separate Windows
platform coverage gate remain required.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
