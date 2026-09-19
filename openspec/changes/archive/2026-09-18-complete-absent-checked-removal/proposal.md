## Why

Windows native CI failed a cold offline provision with `Dolt engine publication succeeded, but private stage cleanup failed -> Rejected removal at ...\.install-jRO6xa\private: The system cannot find the file specified. (os error 2)` (PR #42, run 35382536799 attempt 3, job 105746719916). The chain carries no bounded-recovery exhaustion context, so the failure left `close_windows_private_stage_with` on the very first `remove_tree()`: the classification is wrong, not the two-second window. A Windows directory enumeration still lists a delete-pending entry — here the private copy of `dolt.exe` the cold probe just executed — and once its last handle closes the name disappears, so the per-entry `OPEN_EXISTING` that follows the enumeration snapshot fails with `ERROR_FILE_NOT_FOUND` and `remove_children` maps it through `phase(removed)` to `Rejected`. The object the removal wanted gone is gone, which is exactly the desired end state, yet the cold-start conversation fails.

The same hole exists on unix, where `remove_children` maps an `openat` `ENOENT` after `Dir::read_from` to the same rejection, so the defect is a cross-platform contract gap rather than a Windows accident. The platform layer already honours absence at the end of a tree removal (`remove_tree`'s final absence check, unix `verify_absent`) and the memory layer honours it between attempts (`ChildState::Absent => break`), but neither honours it *during* an attempt.

## What Changes

- In checked tree removal, an already-absent target is completed removal, not a rejection: a per-entry handle open (or the identity read behind it) that reports `NotFound`/`ERROR_FILE_NOT_FOUND` moves to the next enumerated entry, and an absent root or an absent directory name at its deletion open completes that removal, still proven by the existing final absence check.
- Applies narrowly to absence only. Access denied (native 5), sharing violation (native 32), identity/replacement rejections, symlink and junction refusals, depth limits and every uncertain result keep their current meaning and phase, and `removed` is not set by a step that removed nothing.
- No retry policy change: `recoverable_child_removal`, the two-second `CLEANUP_RETRY_LIMIT`, the identity re-checks and first-cause exhaustion in `kuru-memory` are untouched. Once absence is success there is nothing to retry, and native raw error 2 is deliberately *not* added to the memory-layer predicate.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-platform/src/fs/windows.rs` (`remove_tree` root pin, `remove_children` per-entry opens, `remove_empty_directory` deletion open), `packages/kuru-platform/src/fs/unix.rs` (`remove_tree` root `verify_named`, `remove_children` child opens), and `packages/kuru-platform/src/fs.rs` (`RemovalError` contract documentation, a test-only enumeration seam and regression tests). No public API, signature, privacy, retention, timeout, retry or process-lifecycle change; `kuru-memory` is unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
