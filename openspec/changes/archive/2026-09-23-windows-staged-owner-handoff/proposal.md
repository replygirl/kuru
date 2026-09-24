## Why

Windows verified file edits create their shielded staged payload with the process-token user as owner, while an ordinary source file can be owned by the distinct `TokenOwner` under elevation. The access-policy handoff refuses that real owner mismatch before copying the source DACL, so native file edits fail even though both ownership forms are already validated private principals.

One Windows ACL fixture also attempts a DACL mutation through a handle that never requested `WRITE_DAC`, producing an independent access-denied failure instead of exercising the intended policy transition.

## What Changes

- Reopen only the retained staged file object with `WRITE_OWNER` during the Windows access-policy handoff, assign the retained source owner, and revalidate the resulting exact owner and private shielding before copying the source DACL.
- Continue to reject an owner that Windows cannot assign or that is neither the process-token user nor its exact `TokenOwner` with effective OWNER RIGHTS suppression.
- Give the ACL fixture an exact-handle `WRITE_DAC` reopen before changing its test DACL.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-platform/src/windows/security.rs`
- Windows native platform and verified-file-edit acceptance
- No public API, connector behavior, timeout, retry, dependency, or non-Windows change

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
