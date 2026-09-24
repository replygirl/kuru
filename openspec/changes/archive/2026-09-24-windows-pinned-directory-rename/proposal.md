## Why

On native Windows, the existing metadata-only `Directory::open(..., NameRetention::Pinned)` handle allowed both an explicit DELETE open and a pathname rename despite omitting `FILE_SHARE_DELETE`. A prepared `file_list` therefore lost the name-stability boundary its checked read relies on, and its native regression failed when `rename` unexpectedly succeeded.

## What Changes

- Request `FILE_TRAVERSE` for pinned Windows directory opens while retaining the existing metadata/control rights and no-delete-share mode. Movable directory opens keep their current access and sharing.
- Use that same strong pin for checked tree-removal ancestors and inspected directory descendants; continue opening regular-file descendants with their existing rights.
- Add a native production-path regression for the pinned handle, exact object identity and bytes, and same-path operations after release. Keep the prepared-read consumer refusal assertion.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only the Windows directory-open and checked tree-removal boundaries in `kuru-platform` and their native fixtures change. Pinned opens on directories whose ACL denies traversal now refuse explicitly; they do not fall back to a weaker handle. No public API, provider authority, or regular-file-open rights change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (Windows directory handle sharing)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
