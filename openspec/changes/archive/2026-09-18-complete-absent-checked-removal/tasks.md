## 1. Absence completes a checked removal

- [x] 1.1 In `packages/kuru-platform/src/fs/windows.rs`, continue to the next enumerated entry when a per-entry pinned open, movable open or the identity read behind them reports `NotFound`, leaving `removed` untouched; complete `remove_tree` through its existing absence check when the root pin reports `NotFound`; and return success from `remove_empty_directory` when its DELETE open reports `NotFound`.
- [x] 1.2 In `packages/kuru-platform/src/fs/unix.rs`, continue on `Errno::NOENT` for both child `openat` arms in `remove_children`, and complete `remove_tree` through `verify_absent` and the parent `sync_all` when the root `verify_named` reports `NOENT`.
- [x] 1.3 Record the absence-is-completion contract on `RemovalError` in `packages/kuru-platform/src/fs.rs`, including that its `path` always names the tree root rather than the descendant that failed.
- [x] 1.4 Leave `kuru-memory`'s `recoverable_child_removal`, the two-second `CLEANUP_RETRY_LIMIT`, its identity re-checks and first-cause exhaustion unchanged; native raw error 2 is deliberately not added there.

## 2. Regression coverage

- [x] 2.1 Add the `#[cfg(test)]` crate-private `fs::enumeration_seam`, invoked by both backends on each enumerated entry after its name is observed and before any handle on it is opened.
- [x] 2.2 Add `fs::tests::checked_tree_removal_completes_when_an_enumerated_child_vanishes_first` (runs on unix and Windows): an enumerated entry deleted in that window completes the removal, the root disappears and an outside sentinel tree is untouched.
- [x] 2.3 Add the Windows-only `checked_tree_removal_completes_when_a_pending_delete_finishes_after_enumeration`, whose fixture leaves an entry delete-pending and closes the last handle inside the seam window, asserting success rather than `Rejected` with os error 2.
- [x] 2.4 Keep the negative guards passing unchanged: replaced root, pinned root, symlink descendant, junction, rebound names and the held-root uncertain final unlink.

## 3. Verification

- [x] 3.1 Run repository formatting, platform and memory lint, the platform suite, the host-runnable memory `files` and `provision::native_tests` fixtures, and a Windows-target type check; record Windows execution as pending until native CI.
- [x] 3.2 Archive the change before the branch commit.

Observed: `mise run format:check` clean; `mise run //packages/kuru-platform:lint` and `//packages/kuru-memory:lint` passed; `mise run //packages/kuru-platform:test` passed 15 lib + 20 integration + 2 process tests; the new unix-runnable regression passed 3/3 consecutive runs and fails without the fix (`Rejected` from the `NOENT` arm); `mise run //packages/kuru-memory:test -- files` passed 4/4 host-runnable and `-- provision::native_tests` passed 9/9, including `successful_activation_removes_its_disposable_stage`; `cargo check --target x86_64-pc-windows-msvc -p kuru-platform --all-targets` clean apart from a pre-existing unused-import warning in `tests/filesystem.rs`. Every `#[cfg(windows)]` assertion and the CI reproduction remain pending native Windows CI: this host is macOS.

Deviation from the investigation plan, deliberate: no `#[cfg(windows)]` fixture was added in `packages/kuru-memory/src/files.rs`. The seam that makes the race deterministic is `#[cfg(test)]` and crate-private to `kuru-platform`, so a memory-layer fixture could not drive it without exporting a test hook across the crate boundary; the memory layer's policy is unchanged by this fix, and the mis-mapping is asserted at the layer that produces it.
