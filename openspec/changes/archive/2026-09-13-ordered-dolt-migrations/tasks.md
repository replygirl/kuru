## 1. Schema registry and validation

- [x] 1.1 Add a private `packages/kuru-memory/src/store/migrations.rs` registry
  with consecutive immutable definitions, version-2 receipt DDL/digest/IDs and
  version-specific validators; verify unit cases reject gaps, duplicate targets,
  changed receipts, malformed UUIDs and unsupported future versions.
- [x] 1.2 Refactor store initialization and validation to read the stable schema
  row first, validate version 1 or the complete current receipt chain, and keep
  activation/identity/protocol versions independent; verify fresh current stores
  and retained version-1 fixtures select the intended validator without changing
  public memory APIs.

## 2. Isolated attempts and publication

- [x] 2.1 Implement strict bounded reserved-branch naming, enumeration and
  applied-step historical-complete/historical-failed and current-step
  pristine/ready/retained-failed/unresolved classification using branch-local
  `AS OF 'HEAD'` state, registered receipts, ancestry and separate bounded
  status; verify branch-create acknowledgement loss, malformed/excess inventory
  and every ambiguous shape fail or reconcile without reset, deletion or a
  second owner.
- [x] 2.2 Build each step on a new exact-base migration branch, conditionally
  advance version and receipt, call manual `DOLT_COMMIT`, wait for the accepted
  session to end and validate one clean direct-child target; verify pre-commit
  dirty DDL is retained off main and lost commit replies produce no duplicate
  branch, DDL or commit.
- [x] 2.3 Publish only the validated target with exact-base `--ff-only`
  promotion and strict clean base/target reconciliation; verify acknowledged,
  absent and actual lost-reply outcomes preserve active main on every unresolved
  result and accept the target exactly once.

## 3. Startup ownership and cold recovery

- [x] 3.1 Move startup lock, owned server, main pool, migration plan and exact
  base into a worker before `DOLT_BRANCH`, and retain ownership through branch
  pools/connections, original-session teardown, reconciliation, pool closure and
  actual server reaping; verify controlled Tokio cancellation cannot release the
  lock, publish a store to the abandoned caller or let a competing open observe
  intermediate main.
- [x] 3.2 Integrate ordered migration and cold attempt discovery into writable
  `MemoryStore::open_inner` before current-schema publication, while read-only
  opens report old versions and future/inconsistent schemas fail before schema
  mutation; verify stopped persisted-v1 upgrade, second-reopen idempotence,
  process-loss recovery and inspection-owned server contention with real Dolt.
- [x] 3.3 Extend owned cleanup so any bounded `Server::close` handoff retains
  the project startup lock until the supervisor has reaped Dolt; verify native
  child/process interruption cannot release writer authority early on Unix or
  Windows. Transfer that guard through the private server-open path before any
  startup await or owned spawn, including staging and recovery opens.
- [x] 3.4 Separate current read-only and historical inspection from writable
  cleanliness validation while retaining version-specific schema/receipt and
  applicable attempt checks; reject working changes to either authority table
  at every supported version. Verify ordinary dirty reads succeed and dirty
  authority/ref cases fail without changing heads, refs or full working status,
  including a malformed committed v1 receipt table hidden by a dirty drop.

## 4. Staging and historical branches

- [x] 4.1 Run fresh initialization and preserved SQLite import through the same
  ordered registry before format-1 ready publication, and preserve any unready
  stopped stage under the existing interrupted protocol; verify normal,
  marker-loss, migration-failure, server-shutdown and directory-move boundaries
  never activate/reuse dirty state or modify the legacy source/snapshot. Recover
  an already-ready supported-old stage by validating its original captured
  revision and historical attempts, activating its unchanged marker under the
  retained leases, then using the ordinary active-main upgrade path.
- [x] 4.2 Add a crate-private version-dispatching historical branch inspector
  that never migrates or invokes latest-main validation; verify a candidate-only
  version-1 history remains readable with unchanged name/head after main reaches
  version 2, stays unpromotable as stale, and a new current candidate plus later
  writes succeed. Also verify retained version-2 attempt refs do not block reopen
  after later writes or a registered version-2-to-version-3 migration.

## 5. Documentation and native acceptance

- [x] 5.1 Document ordered automatic writable upgrades, read-only/future-version
  behavior, preserved attempts/history and the distinction among SQL schema,
  `ready.json`, identity and supervisor formats in `docs/memory.md` and the
  curated `apps/kuru-docs/concepts/memory.md`; verify `mise run docs:check`
  renders and validates the updated content.
- [x] 5.2 Complete focused named real-Dolt test cases for persisted upgrade,
  wire faults, cancellation, staging and candidates; run those filters through
  `mise run //packages/kuru-memory:test -- <filter>` during implementation and
  verify each leaves no live child/session or fixture artifact.
- [x] 5.3 Run the applicable migration and owner-loss fixtures in the supported
  native Windows job and record actual runtime evidence before merge; verify
  real Windows Dolt/process/lock behavior passes rather than substituting
  cross-target compilation.
- [x] 5.4 Run `mise run format:check`,
  `mise run //packages/kuru-memory:lint` and
  `mise run //packages/kuru-memory:typecheck`, strict-validate this change, then
  run the single coordinated workspace coverage task; verify every command exits
  zero and coverage remains at or above 90 percent without exclusions.
- [x] 5.5 Restore the native Windows acceptance owners' explicit empty reap-guard state for fixture launches that do not install a migration startup lock; verify the target compiles and the existing supervisor lifetime fixtures still complete through `finish_owner`.
