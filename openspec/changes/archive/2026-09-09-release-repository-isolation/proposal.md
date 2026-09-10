## Why

Delivery tools accept an explicit repository root but inherit Git's hook-local
repository environment. During pre-push from a linked worktree, release-note
fixtures ran Git commands against the calling checkout, creating an empty local
commit and changing local repository configuration instead of staying isolated.
The push stopped at its required checks; nothing was pushed.

## What Changes

Remove inherited repository-selection variables from explicitly rooted delivery
subprocesses, including Git, conventional-version tooling and Communiqué. Preserve
deliberate per-command settings such as the private temporary stamping index.
Exercise actual child commands with a poisoned hook environment against temporary
repositories and assert the caller's refs, index and configuration remain intact.

## Capabilities

### Modified Capabilities

None. Existing explicit-root and fixture-isolation contracts are correct.

## Impact

Delivery subprocess construction and real Git/notes integration tests. No new
dependencies, workflow topology, credentials, product configuration or migration.
Restore only the test-created local metadata; preserve installer work in the other
worktree and retain the accidental empty commit under a diagnostic local ref.

## Surfaces

- [ ] interactive
- [x] deploy
- [x] integration
- [ ] agent-behavior
