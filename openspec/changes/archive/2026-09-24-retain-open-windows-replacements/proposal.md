## Why

Windows `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` rejects a checked replacement while the caller retains an open data handle to the validated destination, even when every handle grants delete sharing. Verified file edits deliberately retain that handle so the receipt, original bytes, native identity and access policy stay bound through publication, making ordinary Windows replacements fail with `ERROR_ACCESS_DENIED` before the intended access-policy finalization.

Dropping the destination handle would remove the identity evidence that prevents an unrelated replacement from being adopted between validation and publication. Windows provides a handle-relative POSIX replacement operation that supports this retained-handle contract.

## What Changes

- Use `SetFileInformationByHandle(FileRenameInfoEx)` with replace-existing and POSIX semantics for checked Windows regular-file replacement.
- Derive and retain a `DELETE`-only handle from the exact private staged file before copying a possibly restrictive target DACL, bind it to the opaque copied-access token, and resolve the destination relative to the already-retained destination parent.
- Keep new-only publication on the existing path and preserve preflight validation, source flushing, private staging, access-policy finalization and uncertain post-dispatch errors.
- Exercise a retained destination handle through replacement, proving its old identity and bytes remain readable while the published path resolves to the candidate and inherits later parent access changes.
- Exercise a target DACL that denies file DELETE while its parent authorizes replacement, proving the pre-copy staged authority reaches publication without broadening ordinary source rights.

## Capabilities

### New Capabilities

### Modified Capabilities
- `native-platform`: Clarify that checked Windows replacement retains the validated destination identity through one native handle-relative replacement operation.

## Impact

The change is confined to `packages/kuru-platform` Windows filesystem publication and its native tests, plus the existing `native-platform` capability contract. It adds no dependency, user setting, retry, fallback or cross-volume behavior, and leaves Unix and new-only publication unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
