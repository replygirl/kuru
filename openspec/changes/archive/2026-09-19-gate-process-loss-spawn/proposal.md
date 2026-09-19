## Why

`openspec/changes/archive/2026-09-19-serialize-memory-test-lock-and-spawn`
added a crate-internal, test-only gate (`crate::spawn_gate`) to
`packages/kuru-memory` so real-Dolt fixtures that spawn a child process
cannot transiently hold a duplicate of another test's `flock`'d lock file
across `posix_spawn`'s descriptor-table copy, which was intermittently
flipping `supervisor_rejects_bad_configuration_and_parent_eof_without_spawning`'s
final assertion from `"not been initialized"` to `"parent closed"`. That
change's own description claims a single gated choke point for every
in-binary spawn touched by real-Dolt fixtures, but
`store/recovery_tests.rs`'s `spawn_process_loss_creator` (used by
`ProcessLossChild::spawn`, in turn used by three tests) launches a whole
re-exec'd test-binary child directly via `Command::spawn()` (Unix) /
`NativeSpawnSpec::spawn().await` (Windows) and never takes the gate. A
full-file grep for `spawn_gate` in `recovery_tests.rs` only matches its
existing `crate::test_support::spawn_gated_open` calls, confirming this raw
spawn path was missed. It reopens the exact race the gate exists to close.

## What Changes

Hold `crate::spawn_gate::spawning()` across the `Command::spawn()` /
`NativeSpawnSpec::spawn().await` calls inside `spawn_process_loss_creator`
on both the Unix and Windows branches, mirroring the "held across the spawn;
see `crate::spawn_gate`" pattern already used at every other raw spawn site
this gate covers (for example `server_tests.rs`'s `crate::engine::spawn`
call and `provision/native_tests.rs`'s `gated_cache_lock`). No production
code path changes; no assertions are weakened or retried.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-memory/src/store/recovery_tests.rs`: gate the two
  `spawn_process_loss_creator` implementations (`#[cfg(unix)]` and
  `#[cfg(windows)]`); no other file changes.
- Test-only; no production API, schema, or runtime behavior is affected.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
