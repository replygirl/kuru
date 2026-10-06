# legacy-memory-import Specification

## Purpose
Inspect retained SQLite project memory and explicitly import one validated source scope into Kuru's checked per-project memory store without altering the original source.

## Requirements

### Requirement: Bounded legacy project inventory

Kuru SHALL expose a provider-free, bounded, read-only inventory of project scopes in a validated legacy SQLite source without opening or provisioning each Dolt project. The inventory MUST report the exact opaque source scope and whether the matching canonical project is already imported, suppressed by explicit purge, or unresolved. It MUST distinguish invalid, interrupted, over-bound, and ambiguous source evidence from absence. It MUST preserve source database and committed WAL content, existing project stores, and retained snapshot receipts, and MUST NOT assign a source scope to an unrelated current path. Inventory limits and source changes MUST produce a typed refusal without a partial result or project mutation.

#### Scenario: Multiple scopes and committed WAL
- **WHEN** a legacy source contains several project scopes and committed WAL rows
- **THEN** one consistent bounded source view reports each supported scope and its status without activating any project or losing committed WAL data.

#### Scenario: Invalid, changed, or over-bound source
- **WHEN** the source is malformed, incomplete, changes during observation, or exceeds an inventory bound
- **THEN** inventory reports a bounded typed refusal with no partial success list, project mutation, or retained snapshot.

#### Scenario: Existing, suppressed, and unresolved targets
- **WHEN** inventory observes an active canonical target, explicit purge suppression, or a target whose state cannot be safely determined
- **THEN** it reports that state without reimporting, clearing suppression, or treating uncertainty as absence.

### Requirement: Explicit source-selected legacy import

`kuru memory import` SHALL select exactly one canonical destination project. It MAY default to the exact matching source scope; importing a different scope MUST require an explicit source selection from inventory and MUST record the source-to-target mapping in the durable activation receipt. Import MUST transform only the leading project scope of validated message namespaces and state keys, preserving order, content, and admitted state JSON exactly. For moved-project imports, an unrecognized state-key family, identity-bearing payload, transformed-key collision, row extending the source scope without its separator, or unproved source scope MUST refuse before activation. Same-scope import MUST retain existing compatibility. Automatic first-open import MUST continue skipping rows outside the exact canonical scope.

Import MUST use the existing checked snapshot and schema activation path, retain the original source and a consistent committed snapshot, refuse active destinations and explicit purge suppression, and return only bounded result metadata (source scope, target scope, counts and activated revision), not private rows. A definite no-effect refusal MUST carry a typed refusal kind and MUST leave no new snapshot or candidate in `memory/legacy`.

#### Scenario: Exact-scope import succeeds
- **WHEN** the selected source scope equals the canonical destination, the source is valid, and the destination is absent
- **THEN** only that scope's validated rows are activated once in original order, opaque content is preserved, and the receipt identifies the source, destination, snapshot, and revision.

#### Scenario: Explicit moved-scope import succeeds
- **WHEN** the user explicitly selects one inventoried source scope for a different canonical destination
- **THEN** only the defined leading scope is remapped, admitted JSON and transcript content remain byte-identical, and both scopes are retained in the receipt.

#### Scenario: Ambiguous remapping refuses
- **WHEN** a foreign scope was not explicitly selected or its rows cannot be proven safe to remap
- **THEN** import refuses before activation and does not relabel or copy another project's state.

#### Scenario: Collision, suppression, or active target refuses
- **WHEN** the target already contains memory, is explicitly suppressed, has a transformed-key collision, or has ambiguous retained import state
- **THEN** import reports a typed no-effect refusal and preserves the target, source, snapshots, and suppression state.

### Requirement: Import admission excludes active and draining drivers

Explicit import MUST acquire the existing native session-maintenance barrier and checked owner maintenance permit before creating staging data, and MUST recheck destination admission after ownership is acquired. If an admitted session or draining driver prevents maintenance, import MUST return a typed no-effect refusal before staging or source snapshot publication. Definite native lock contention MUST be distinguished from other operating-system errors without parsing error text. The maintenance permit and any acquired owner resources MUST remain alive through source/database close and owned service cleanup/reap even when the requesting future is cancelled; uncertain cleanup MUST remain represented as uncertainty and MUST NOT be converted into a success or destructive retry.

#### Scenario: Admitted or draining driver blocks import
- **WHEN** another session is admitted or draining for the selected destination
- **THEN** import refuses before staging and leaves source, destination, suppression and snapshot inventory unchanged.

#### Scenario: Caller is cancelled during owned import work
- **WHEN** the requesting future is dropped after an owned maintenance worker begins
- **THEN** the worker retains maintenance ownership through local connection close and service cleanup/reap before releasing admission, and the caller-visible outcome remains uncertain unless durable reconciliation proves the result.

### Requirement: Consistent snapshot preserves original files

Inventory and explicit import MUST open legacy SQLite read-only, bound observation and snapshot work, and preserve the original database and committed WAL bytes. Snapshot consistency MUST include committed WAL data; SQLite shared-memory coordination files MAY be rebuilt and MUST NOT be represented as byte-preserved. The backup progress deadline MUST reset after each observed progress step, so a large but progressing snapshot is not rejected by a whole-copy timeout.

#### Scenario: Snapshot includes WAL and retains originals
- **WHEN** a committed transaction remains in the legacy WAL during inventory or import
- **THEN** the consistent snapshot includes that transaction while the original database and WAL bytes remain unchanged; any SHM rebuild is disclosed accurately.

#### Scenario: Long progressing backup
- **WHEN** a large snapshot continues making progress beyond thirty seconds overall
- **THEN** the copy continues; only a full no-progress interval may trigger the bounded stall refusal.
