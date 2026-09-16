# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Siblings

P1 typed-message foundation is being implemented in a separate worktree and is
not a cospec-visible change here. It is an integration-ordering soft dependency:
both changes mechanically update `ModelInfo` constructors, but neither P2's
catalog/configuration contract nor its tests need P1's message representation.
