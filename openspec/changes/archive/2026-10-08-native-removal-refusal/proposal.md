# Proposal

## Why

The fresh platform report leaves native unlink rejection uncovered. A checked
owner-private file removal denied by the parent directory's write permissions
must report rejection and preserve the exact file and adjacent state.

## What Changes

- Add a Unix native contract test in `packages/kuru-platform/src/fs/unix.rs`
  using a retained private parent, actual 0500 permissions and checked removal.
- Restore the exact retained parent's permissions before assertions or cleanup,
  then verify rejection phase, error, identity, path, retained bytes and a
  successful retry under restored authority.
- Simplify the existing Unix listener regression in `src/unix.rs` by capturing
  native observations and asserting them after explicit child cleanup, rather
  than translating failed assertions into synthetic I/O errors.

## Impact

Test-only platform coverage expansion. Production APIs, permissions, timers,
dependency pins, source inventory and the canonical 95% line gate are unchanged.
