## Why

Kuru can create, replace and delete project files, but those mutations have no durable file recovery record and no context-verified edit operation. A stale model edit can replace user changes, and a canceled or interrupted write may leave the user without a reliable way to identify or undo the file effect. Phase 2 needs bounded recovery for Kuru-managed file mutations while keeping shell and MCP effects outside that promise.

## What Changes

- Add `file_edit` with ordered exact-context hunks. It rejects stale, ambiguous or overlapping matches before changing any file, then uses the same checked publication path as `file_write` and `file_delete`.
- Before each admitted Kuru file mutation, reserve bounded checkpoint capacity and durably record its operation/turn identity, checked target identity and before/after snapshots in private per-project storage outside the tool root. An interrupted operation keeps a recoverable receipt; exact retries inspect it instead of replaying a mutation blindly.
- Add minimal checkpoint inspection, selected file undo and explicit pruning through CLI/TUI. Undo requires normal project and file authority, rechecks the selected edit's expected post-state, and refuses to overwrite later user changes. An uncertain delete without publication proof remains unresolved; the user can inspect it and explicitly discard an inactive recovery record, forfeiting undo, but absence alone never authorizes restoration.
- Document snapshot and total-capacity limits, no automatic expiry, retained originals and the distinction from dream undo, chat rewind and arbitrary shell/MCP effects.

## Capabilities

### New Capabilities

- `verified-file-edits`: Checked edit application, private bounded file checkpoints, exact-effect recovery, selected undo and explicit pruning.

### Modified Capabilities

- `provider-tools`: Native file mutations participate in the checked checkpoint path and preserve their normal permission/protected-path decisions.
- `command-registry`: Working file checkpoint inspect/undo/prune commands share the advertised registry when their backend is available.

## Impact

- `packages/kuru-connectors` owns the file tools and common checked mutation path; `packages/kuru-platform` supplies retained filesystem identity and publication primitives; `packages/kuru-core` owns bounded edit/checkpoint request types. `packages/kuru-runtime` supplies admitted turn/call identity, and `apps/kuru-tui` supplies the private data directory, command surface and notices.
- Checkpoints occupy a private project-bound directory separate from Dolt memory and inaccessible under the workspace tool root. Existing write-capable constructors must receive this store before advertising file mutations; read-only tools and unrelated shell/MCP authorities remain unchanged.
- No Dolt schema migration or whole-workspace snapshot. This change does not claim secure erasure, arbitrary-process rollback or automatic deletion of retained checkpoint records.

## Surfaces

- [x] interactive — file edit, checkpoint inspection, selected undo and pruning
- [ ] deploy — no runtime process topology change
- [x] integration — checked native filesystem and private data-dir publication
- [x] agent-behavior — `file_edit` and file tool results/authority change
