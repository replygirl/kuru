# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Coordination, not a block

Neither item below is a change slug that exists on this branch, so cospec cannot ledger it (a dangling slug fails `cospec validate --strict` with `blockers/dangling-ref`). Both are tracked here and in `tasks.md` task 1.2 instead.

- PR #207 (`fix/unix-spawn-pipe-inheritance`, open, another assistant; change `unix-spawn-pipe-inheritance`, archived only on that branch) edits `packages/kuru-connectors/src/hooks.rs`, `src/rpc.rs` and `src/unix_shell.rs`. This change edits none of the three. Read-only `gh pr diff 207` on 2026-10-04: its `rpc.rs` hunks are the imports (about lines 8-22) and the `Session` spawn (about 389-400); its `unix_shell.rs` hunks are the imports and the shell worker spawn (about 827-846); its `hooks.rs` hunk is the hook spawn (about 1001-1020). None touches the `Step::Sleep` call sites at `rpc.rs:1000` and `:1043`, which c1#7 leaves in place, or the `wait_for_requests` caller at `rpc.rs:1078`, whose behaviour c1#64 changes through the helper in `src/test_support.rs` without editing `rpc.rs`. Because #207 adds three import lines to `unix_shell.rs`, the restated `CLEANUP_ALLOWANCE` citation (`unix_shell.rs:36` on 449dca9e) moves to about `:39` after it merges (inference, not rehearsed); the value is unchanged. #207 also isolates owned spawns from each other's stdio pipes, which should make the `Step::Park` peer's stdin EOF prompt; without it the product's `Rpc::close` still signals the group after `GRACE` (`rpc.rs:847`), so a parked peer is reaped either way (inference from the close path, not measured).
- PR #212 (`test/tui-derived-waits`, open; change `tui-derived-waits`, active only on that branch) waits for this change to land. Its CI hit c1#129 (run 37180709856, macOS partition 2, job 111373013191, "isolated MCP environment fixture timed out"), a site this change fixes. It edits no kuru-connectors file (its connectors-related text restates `IO_TIMEOUT` in `apps/kuru-tui` test support), so the two do not overlap in code.
