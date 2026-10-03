# Proposal

## Why

The one-time legacy SQLite import in `kuru-memory` copies the old database with SQLite's online backup API under a single 30 s deadline that is checked on every loop iteration, including iterations where the backup step made progress. A large legacy database or slow storage therefore fails the import with zero contention once the whole copy exceeds 30 s, and the failure message ("close older writers and retry") blames writers that may not exist, sending the user after a cause that is not there.

## What Changes

- The backup loop in `migration::prepare` moves into a file-local function, `run_backup`, generic over the step closure and a clock closure; production passes `|| backup.step(256)` and `Instant::now`.
- The bound is named `SNAPSHOT_STALL_BOUND` (value unchanged, 30 s) and restarts on every `StepResult::More`, so it covers only a stall in which the source stays busy or locked. A copy that keeps progressing completes however long it takes.
- The failure names the stall and is formatted from the constant: "legacy memory snapshot made no progress for 30 s while the source was busy or locked; retry when it is free".
- The existing 10 ms pause after a busy or locked step is unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-memory/src/migration.rs` and its in-file tests only.
- No public API, configuration, storage format or documentation change; the message and the 30 s bound are not documented in `docs/`.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
