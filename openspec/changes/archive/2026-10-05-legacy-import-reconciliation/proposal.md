# Proposal

## Why

Kuru can migrate an exact matching SQLite scope during first open, but users cannot safely inspect or deliberately import other retained project scopes. The preserved-source import path must work with the current concurrent-session owner model and must refuse before leaving staging artifacts whenever the destination is active, suppressed, ambiguous, or otherwise not provably safe.

## What Changes

- Add bounded provider-free inventory and explicit source-to-canonical-project import commands.
- Preserve the original SQLite database and committed WAL data while constructing a validated consistent snapshot; disclose that SQLite may rebuild SHM coordination state.
- Bind import to checked maintenance admission, recheck destination state after admission, and retain ownership through local connection close and service cleanup even if the requesting future is dropped.
- Return bounded typed no-effect refusal kinds and retain the current progress-resetting snapshot stall bound.
- Preserve ordering, opaque content/state values, and durable source/target receipt data.

## Capabilities

### New Capabilities
- `legacy-memory-import`: bounded inventory and explicit, checked import of retained SQLite project memory.

### Modified Capabilities

## Impact

Changes are confined to `kuru-memory` migration/facade/store/service/session-driver seams, the `kuru memory` CLI and focused tests, and command/privacy documentation. No schema or dependency-version pin changes are required; the existing workspace-pinned `rusqlite` enables its `hooks` feature for a progress-resetting backup bound. No provider, credential, or mise changes are made. The existing archive/source preservation format and current schema remain unchanged.

## Surfaces

- [x] interactive — explicit inventory/import commands and refusal output
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
