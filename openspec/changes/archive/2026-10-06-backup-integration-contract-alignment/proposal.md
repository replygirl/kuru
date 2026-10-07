# Proposal

## Why

M1's staged restore commit uses a quoted `-m` message option that the existing
file-wide destructive-branch scanner intentionally refuses. Its historical
preferences fixture also expects another project's defaults through the original
project's bound view, conflicting with the retained canonical-project authority.
Original native CI reported both contract mismatches before merge.

## What Changes

- Use the already established `--message` spelling for the staged DOLT_COMMIT
  option, preserving the working snapshot option, SQL, binds and root checks.
- Assert foreign-project refusal through the bound preferences view, then retain
  the original defaults assertion through a separately correctly-bound store and
  explicitly close it.
- Retain every branch-scanner rule and original preference lifecycle assertion.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing backup and canonical-project authority contracts are correct.

## Impact

Only `packages/kuru-memory/src/store/backup_restore.rs` and
`packages/kuru-runtime/src/preferences_tests.rs` change. No API, protocol, schema,
budget, dependency, workflow or authority changes. Required acceptance is the
existing branch scanner/negative shapes, dirty remapped native restore, corrected
preferences case/closing guard and relevant package static checks.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — existing native Dolt message option and bound store contract
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
