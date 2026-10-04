# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Coordination

PR #207 (`fix/unix-spawn-pipe-inheritance`, change `unix-spawn-pipe-inheritance`, archived inside that branch) is open on the same package. It edits `packages/kuru-platform/src/unix.rs`, `src/unix/snapshot.rs`, `tests/unix_process_group.rs`, `packages/kuru-platform/Cargo.toml`, `AGENTS.md`, `docs/development.md` and the `native-platform` spec. This change owns `tests/windows_process.rs`, `tests/fixtures/process.rs` and its own change directory, so no file is shared and the two merge in either order. No active change on this base touches those test files.
