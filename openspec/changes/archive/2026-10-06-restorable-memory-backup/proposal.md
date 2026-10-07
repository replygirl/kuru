# Proposal

## Why

The active-memory export describes application records at one revision, but cannot restore a project's complete Dolt history, retained candidates, working sets and operational usage ref. Users need a checked offline recovery image that remains usable across supported schema upgrades without stopping independent conversations while it is captured.

## What Changes

- Add provider-free `kuru memory backup`, `verify` and `restore` commands. Backup captures one native Dolt dataset root through the existing project owner while writers remain admitted; verification proves both exact physical inventory and native restorability.
- Publish only a private, independently validated image with a bounded manifest of its completed root, captured schema/runtime provenance and actual refs. Include all native reachable history and working sets, including retained candidates and the same-dataset usage ref; exclude external credentials, grants, claims, endpoint records and runtime caches.
- Restore only to an absent explicit managed target, under the existing exclusive maintenance and lifecycle ownership. Require explicit canonical-root remap when roots differ, mint new live store/service authority, preserve historical namespace and SQL-origin receipts, and migrate a supported historical schema in unpublished staging before activation.
- Keep existing operational deadlines, checked process cleanup and uncertain outcomes. Drain native output to EOF with bounded retention; cancellation or ambiguous publication never proves completion or permits deleting a live stage.

## Capabilities

### New Capabilities

- `project-memory-backup`: Native complete-history backup, independent image verification and compatible absent-target restore.

### Modified Capabilities

- `project-memory-service`: An owner-served backup operation retains accepted-work ownership without excluding ordinary writers or granting SQL/control authority.
- `versioned-memory`: Restored activation preserves original history/provenance while assigning fresh target authority and performing supported staged migrations.

## Impact

Owning files are `packages/kuru-memory/src/backup.rs`, store/server identity and staging paths, facade/service RPC and their fixtures; `packages/kuru-platform` only where a checked native launch is necessary; `apps/kuru-tui/src/cli.rs` Memory subcommands and their tests; and owning memory/user documentation. The private service protocol and exact wire pin advance together. Activation/identity records gain backward-compatible optional restore provenance; no application SQL schema, dependency pin, provider route, session admission policy or durable live-registration table changes are planned. The old P20 worktree is read-only context, not a merge source.

## Surfaces

- [x] interactive — explicit CLI backup, verify, restore, remap and failure output
- [x] deploy — native offline engine validation, staged activation and owned cleanup
- [x] integration — pinned Dolt native backup/restore/fsck and supported image/schema compatibility
- [ ] agent-behavior — no prompt, peer, provider or model-policy change
