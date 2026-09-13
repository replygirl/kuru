## Context

Current `MemoryStore::open_inner` accepts only schema version 1 and validates
main before returning a store. It already holds a checked per-project startup
lock through server startup and staging activation; the supervisor separately
holds the server lifecycle through Dolt reaping. `Candidate::promote` provides
an exact-base fast-forward and base/target reply-loss reconciliation shape.

The prerequisite pinned-Dolt proof establishes two distinct facts. Manual
`DOLT_COMMIT('-Am', ...)` can publish DDL plus version/receipt-style DML as one
clean commit, including after reply loss. A disconnect before that procedure
leaves `HEAD` unchanged but retains `CREATE TABLE` as a dirty working-set change.
Therefore migration DDL cannot execute on active main.

## Goals / Non-Goals

**Goals:**

- Upgrade persisted compatible schemas in order while active main always names
  one complete clean version.
- Make every accepted attempt discoverable and reconciliable after cancellation,
  connection loss or process loss.
- Preserve old main history, candidates, legacy sources and staging evidence.
- Keep engine-specific mechanics private to `kuru-memory` and exercise them on
  every supported native platform.

**Non-Goals:**

- Turn journaling, replay, candidate reclamation, history expiry, selective
  erasure, role/facing policy or new public branch-management APIs.
- A generic migration language, online concurrent writers, directory-copy
  upgrades, engine setting changes or cleanup by reset/delete.
- Changing `ready.json`, identity, supervisor protocol or existing operation
  receipt formats solely because the SQL schema advances.

## Decisions

### Immutable one-step registry and committed receipts

A private registry entry contains consecutive `from` and `to` versions, a
permanent ID, exact ordered migration SQL and data-transform identity, a stable
postcondition identity, the exact recognized failed-status tuples, and any
receipt-protocol discriminator needed to interpret them. Its SHA-256 definition
digest binds exactly those values with unambiguous field boundaries. It
does not claim to hash Rust code or automatically detect an arbitrary validator
implementation change. The generic conditional schema-version update, receipt
insertion, session teardown and Dolt publication sequence remains fixed runner
protocol. Version 2 creates `kuru_migrations` and records its own target
version, ID, digest and operation UUID while conditionally advancing the single
`kuru_schema` row. The target version is the receipt primary key and the
operation UUID is unique. Validation requires exactly the expected ordered
receipt set through the branch version; missing, extra or reordered versions
fail closed. Version 1 must not already contain the reserved migration receipt
table; a future receipt authority cannot be accepted as a pristine source.

The migration target's direct parent remains the source-revision authority, so
the receipt does not duplicate source/target revisions or candidate inventory.
Changing a released definition requires a later migration. A mutable registry
or filesystem-only receipt was rejected because either could disagree with
versioned schema history.

### Reserved exact-base branches isolate DDL

An attempt name encodes the target version and a simple lowercase UUID under a
strict `kuru_migration_` namespace. The worker calls `DOLT_BRANCH(name, base)`
with the captured clean main head and verifies the resulting ref before opening
its branch pool. All DDL and data changes occur there. `DOLT_COMMIT('-Am', ...)`
creates one candidate commit, which must be clean, have `base` as its direct
parent, match the name's UUID receipt, and satisfy the target validator.

Changing `@@autocommit` was rejected because pinned go-mysql-server commits DDL
implicitly regardless of it. Enabling `dolt_transaction_commit` was rejected
because it can advance branch HEAD for DDL before version/receipt DML. In-place
main DDL was rejected by the native failure tuple. A stopped-directory copy/swap
was rejected because it adds a second whole-store publication and recovery
protocol when existing branch isolation and fast-forward suffice.

### Attempt discovery is bounded and state based

Under startup/lifecycle ownership, enumerate `dolt_branches`, enforce a total
limit of 64 reserved refs (including a proposed new attempt), strictly parse
every reserved attempt name, and
inspect each through its own pool using committed `AS OF 'HEAD'` queries and
immutable revision-addressed pools for complete historical schema validation,
plus separate `dolt_status`. Names for unknown steps, malformed UUIDs or impossible
target versions are unresolved rather than ignored. Classify the inventory in
two passes.

For an already applied step, a clean branch is historical completed only when
its committed receipt and registered step validate, its sole parent validates
as that step's `from` version, and its head is an ancestor of current main.
Advancing that retained branch to a later conversation commit does not turn the
later commit into a completed migration. A dirty branch is historical failed only when its committed
head validates as the registered `from` version, that head is an ancestor of
main, and its bounded status exactly matches that step's registered table,
staged-state and status tuples. These historical branches remain unchanged and do not compete with
the next step. Every other old-step shape is unresolved.

For the one next step at the current base, classify:

- pristine: clean base head, `from` schema, no target receipt; reuse it;
- ready: clean one-commit target with exact parent, receipt and postconditions;
  publish it without replay;
- retained failed: the exact base head with a dirty working set after its
  session is absent, where the bounded status exactly matches the step's
  registered failed-status tuples; leave it untouched and create one new unique
  attempt; or
- unresolved: every other shape, multiple pristine/ready targets, malformed
  reserved names or excessive inventory; fail closed.

No attempt for a later unregistered or out-of-order step is eligible. The
reserved ref and its state are the durable attempt record. A separate
pending file or partial commit was rejected as duplicate authority. Retained
failed and successful branches are never reset or deleted by open; future
bounded reclamation is separate.

A branch-creation error or lost reply is reconciled by exact name and head
before another attempt is created. The production helper must wrap creation in
the same accepted-operation discipline even though the prerequisite fixture
only establishes raw engine publication behavior.

### Publication reuses promotion invariants with stricter validation

Immediately before publication, revalidate clean main at the exact base and
`from` schema. Install pending base/target state, call
`DOLT_MERGE(branch, '--ff-only')`, drop the original socket and wait until that
connection ID disappears. Reconciliation accepts only clean main at `base` or
at the already-validated `target`; target additionally receives full current
schema/receipt validation. A base outcome may retry the same ready target.

Existing `Candidate::promote` is the implementation pattern, but its current
base/target-only reconciliation is insufficient by itself for schema migration.
The migration helper must add clean-status and schema/receipt checks. It must
not make migration branches public candidates or relax stale-candidate rules.

### The worker owns acceptance through teardown

The startup file enters a crate-private guarded server-open path before its
first await or owned child spawn. The ordinary public server-open API remains
unchanged. The shared guard is already present when an `Owner` is constructed,
so cancellation during server startup follows the same independent reaper as
cancellation after SQL acceptance; there is no post-open guard-install gap.

Before `DOLT_BRANCH`, `open_inner` transfers the startup lock, owned `Server`,
main pool, plan and expected base into a spawned worker. The worker creates or
recovers branch pools and owns every mutation connection. Dropping the awaiting
caller detaches rather than aborts accepted work. The worker completes session
teardown, reconciliation, validation, pool closure and actual server reaping
before releasing the startup lock, and never constructs a `MemoryStore` for an
abandoned caller.

A surviving caller receives the still-held lock only after teardown, reopens
the server, revalidates current main, constructs `Shared`/`MemoryStore`, then
releases the lock. If bounded synchronous close hands the child to a background
reaper, the startup-lock guard must move with that reaper. Process death relies
on the supervisor lifecycle lease; cold open waits for reaping before attempt
classification. Merely dropping the current `open_inner` future was rejected
because it cannot guarantee cleanup of an accepted DDL session.

### Staging uses the registry before activation

Fresh and SQLite-import paths keep their existing unpublished private stage:
commit released v1 initialization/import, execute the same ordered branch
migrations, validate current schema while the staging server and SQL session
remain owned, then publish the existing format-1 `ready.json`, stop/reap and
activate the directory. No SQL validation is deferred until after server
shutdown. A live failure stops the
server and moves the unready stage under the existing checked `interrupted/`
protocol. After process loss, cold staging recovery performs the same move. A
markerless stage is preserved even if its SQL happens to look complete; recovery
does not invent the missing activation decision.

An already-ready stage from an older binary is a different accepted state.
Validate its supported found-version schema and complete receipt chain, clean
main, exact captured `initial_revision`, bounded attempt inventory and historical
attempts through that found version. A reserved attempt for a newer version is
inconsistent with that ready decision and fails closed. After actual quiescence,
activate the original stage under the retained lifecycle and startup locks,
preserving its marker bytes; the ordinary active-main runner then performs any
missing upgrades. Recovery neither rewrites the old activation record nor
requires it to have been authored by the current schema version.

### Historical readers dispatch by branch schema

A crate-private inspector reads only `kuru_schema.version` first, selects a
retained immutable validator/reader for that version and rejects unknown future
versions. Global current-schema validation remains scoped to active main. Old
candidates remain unchanged and stale after main advances; new candidates
inherit current schema. Rewriting old branches was rejected because it changes
accepted history and can invalidate their original base relation.

### Inspection validates authority without requiring unrelated data to be clean

Current-schema read-only opens use a distinct private validator that retains
version-specific schema/receipt validation and bounded historical-attempt checks.
Historical readers retain their branch-local version validator. Both reject any
`dolt_status` change to either `kuru_schema` or `kuru_migrations` at every supported
version, but permit unrelated ordinary dirty data or DDL without mutation.
Version 1 separately requires the receipt table to be absent; its status must
still be checked so a dirty drop cannot hide a malformed committed receipt table.
Native fixtures compare heads, refs and full working status before and after both
successful reads and rejected authority changes. Writable opens, migration
construction, ready stages and publication keep their existing clean-state rules.
Reusing the writable validator for inspection was rejected because ordinary
uncommitted data must not turn an otherwise valid read into a migration failure.

## Integration contract

`kuru-memory` remains the sole owner of the Dolt integration. It invokes the
pinned server through existing authenticated branch pools addressed as
`kuru/<validated-branch>` and uses only `DOLT_BRANCH(name, base)`, manual
`DOLT_COMMIT('-Am', ...)`, `DOLT_MERGE(name, '--ff-only')`, `DOLT_HASHOF`,
`dolt_branches`, `dolt_log`, `dolt_commit_ancestors`, `dolt_status` and committed
`AS OF 'HEAD'` queries needed by the decisions above. No migration SQL, branch
name or raw pool crosses the public memory API.

Schema versions are bounded integers. Permanent migration IDs are bounded
ASCII, definition digests are canonical lowercase SHA-256, operation identities
are UUIDs, and reserved branch names encode the target version plus the UUID's
lowercase simple form. Parse and validate these types before interpolating any
identifier; continue binding data values. Receipt identity must round-trip
exactly between the branch name, committed row and compiled definition.

Native fixtures run the bundled pinned Dolt through package-owned mise tasks.
Connection-loss cases extend the existing bounded MySQL wire proxy and native
child controls only in test code. Production receives no fault environment
variable, alternate engine route, download, host fallback or new dependency.

## Risks / Trade-offs

- **[Risk] Repeated interrupted DDL leaves reserved branches.** → Preserve each
  attempt, classify applied-step branches through registered receipts and
  ancestry, bound the total inventory and fail closed at the limit; define
  deletion only in a later operational-reclamation change.
- **[Risk] An accepted worker outlives its caller.** → Transfer all server,
  connection and lock ownership before first mutation and prove the later open
  cannot acquire the lock before real session/server teardown.
- **[Risk] A reply is lost after branch creation, commit or fast-forward.** →
  Reconcile exact names, connection absence, heads, receipts and clean status;
  never infer success from a network result alone.
- **[Risk] Old schemas diverge from latest validation.** → Keep immutable
  version-specific validators/readers and exercise committed v1 candidates after
  main reaches v2.
- **[Risk] Platform process/lock behavior differs.** → Require the same real
  persisted-store, cancellation and interruption fixtures on supported native
  Unix and Windows before merge; cross-compilation is not runtime evidence.
