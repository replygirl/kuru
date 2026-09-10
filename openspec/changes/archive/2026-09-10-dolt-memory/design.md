## Context

Kuru currently stores all projects in one SQLite file. Project namespaces are
canonical-path hashes; private part and relationship histories already have
separate keys. Four production consumers use the small concrete MemoryStore API.
Dreaming currently persists intermediate work directly. Full Dolt 2.3.3 has been
proven locally with authenticated SQL, branch-qualified databases, transactions,
version commits, fast-forward promotion and owned-process shutdown. Gas City's
ownership and migration mechanisms inform this design; neither reference's fleet
or Beads architecture is needed.

## Goals / Non-Goals

Goals: full Dolt for live memory, preserved development data, branch-isolated
dreams, inspectable revisions, compiler-free provisioning and meaningful actual
database tests. Keep existing peer roles, private namespaces and partial proposal
rejection semantics. Non-goals: remote sync, DoltHub, generic backend plugins,
per-peer database branches, arbitrary SQL tools, a fleet daemon or new releases.

## Decisions

### Package and configuration

Add packages/kuru-memory for managed installation, server supervision, async SQL,
version operations and legacy import. Core retains types/configuration; runtime
retains cognition policy. Use exact SQLx 0.9.0 with MySQL and Tokio, without build
time database macros. Use a concrete cloneable MemoryStore with an immutable
branch-selected view, rather than a storage trait or alternate live backend.
Configuration exposes an optional explicit Dolt executable, cache path, offline
policy and bounded startup timeout. Explicit executables still require the exact
supported version. Fresh help/config/version/update do not provision memory.

### Managed runtime

Pin official full-Dolt 2.3.3 assets and both archive/executable digests for four
release platforms. The measured archive has exactly two ordinary directories,
one executable and LICENSES. A package-owned extractor validates exact entries,
types, lengths and bounded decompression before atomic cache activation. Preserve
LICENSES. Never widen the existing Kuru update archive contract. Lock installation
with a stable inode; reject links and corrupt cached executables before execution.
Existing valid caches support offline operation; no SQLite fallback is allowed.

### Private owned sidecar

Use one private Dolt database/server per canonical project. Generated loopback
credentials, explicit data/config paths and a private Dolt home isolate ambient
configuration. Disable version checks and metrics before the version probe;
disable event scheduling with OFF (DISABLED fails in the pinned engine).
Readiness verifies authenticated project/instance identity, database and datadir.
Use small branch-qualified pools and finite startup/query/shutdown deadlines.

A hidden Rust supervisor in the Kuru executable owns and reaps the actual Dolt
Child. A parent-held pipe supplies configuration then remains open as its lifetime
signal. EOF, including parent SIGKILL, makes the supervisor terminate and reap its
owned child before releasing its stable lifecycle lock. No stale PID, occupied
port or unverifiable process authorizes a kill or lock deletion. Readers can
attach to a verified writer server but acquire no ownership. A package-owned
maintenance binary provides the identical supervisor for library integration tests
and engine prefetch; it is not an additional release asset.

Writable opens require lifecycle ownership. A writer waits for a cold inspection's
owned server to stop rather than borrowing its lifetime; independently opened
writers cannot bypass serialization. CLI commands await cleanup on success and
failure before releasing their project writer lease. Directory activation and
preservation hold the supervisor's existing lifecycle inode through rename and
directory fsync. Both the mover and waiting supervisors revalidate directory and
lock identity, so neither can operate on a replacement path.

### Data and revisions

Each project database preserves existing full namespace strings and byte-sensitive
keys. Store JSON as text, not a normalizing SQL JSON type. Preserve message order
and identifier validation. Serialize short mutation transactions locally; never
hold one across inference. Each committed mutation has an operation ID and a Dolt
revision. Each mutation uses a dedicated SQL session. After an uncertain reply,
drop its socket and wait until its connection ID leaves Dolt's process list before
checking the operation record; absence of a receipt while that session can still
commit is not rollback. Promotion similarly records its base/target. A bounded
unresolved outcome blocks subsequent mutations. Branch views remain pinned across
pooled reconnects, with weak pool registries so retired views release connections. No dynamic
user-provided SQL identifiers or raw credentials enter SQL/log output.

### Legacy import

Acquire the existing project writer lease before migration, excluding older Kuru
writers for that project. Read SQLite through its read-only backup API, including
WAL-visible committed data, and preserve a consistent full snapshot plus original.
Validate application/schema identity and data before activating anything. Import
the current project's exact namespace prefix, preserving sequence and opaque JSON;
other projects remain in the preserved source/snapshot because hashes cannot
recover paths. Record source digest, counts and committed revision. Build a staged
project store, validate imported data, close/reap it, then atomically activate the
directory. A completed interrupted stage is reused only when its validated revision
and preserved source match. Partial/superseded stages are stopped and preserved in
an interrupted directory before retry; unknown stages fail explicitly. Never
interpret damaged or unrecognized data as an empty database. Original files are
never removed, overwritten, or used as the live backend.

### Dream and cancellation semantics

Capture live revision and create a unique candidate. Actor Work carries its memory
view so every dream input, response, note, receipt, report and topology update
uses that candidate. Individual model/proposal errors remain report rejections.
Storage errors and cancellation before promotion leave active memory intact, as
does stale-base rejection. An accepted atomic promotion may complete after caller
cancellation; reconcile its durable result before further runtime work.
Commit the complete candidate, then promote only from its expected live base.
Keep abandoned candidates inspectable; do not delete a branch with outstanding
actor writes. Construct topology/config/session candidates in locals, persist
before publishing fields, and do not await between confirmed persistence and
publication. Reconcile accepted writes after cancellation before further mutation,
including in-turn tool calls and turn completion. Refuse to replace an unresolved
publication record with another proposed state.
Undo creates a compensating revision restoring membership and archiving additions;
it preserves later chats, notes, preferences and external tool effects.

## Operational surface

Run native on the host/CI runner, with no container service. Dolt 2.3.3 supports
macOS arm64/amd64 and Linux arm64/amd64. Bind only an allocated 127.0.0.1 TCP
port, with no Unix socket, remotes, MCP or metrics listener. Generate local
database credentials in private storage; no user-provisioned secret is required.
Use at most four SQL connections per view and bound test server concurrency.
The Kuru supervisor owns startup/shutdown; an attached reader does not own it.

## Integration contract

SQLx speaks the pinned engine's MySQL protocol through authenticated branch-
qualified connections. Project scope remains the existing canonical-path SHA256;
part and relationship UUID construction is unchanged. Memory messages retain
role/content and chronological order. Binary SQL keys preserve byte identity;
JSON text preserves existing values. Schema identity is validated on open.
The package owns isolated real-engine fixtures and a supervisor test executable;
runtime tests exercise the same public API with fresh project stores. SQLite is
only a read-only import dependency. No provider, MCP or A2A route changes.

## Risks / Trade-offs

- Full Dolt adds disk, startup and process cost. Keep pools and test server
  concurrency bounded; package-owned prefetch avoids repeated downloads.
- A pinned runtime requires deliberate checksum/license refresh with upgrades.
- SQLite import retains extra disk usage by design. Users can separately archive
  the preserved source after verifying migrated projects.
- If supervisor and parent both crash, fail closed on a held Dolt lock rather
  than guessing process ownership. Document recovery and prove normal crash cleanup.
- Revisions are history, not an off-machine backup. Document a stopped-store copy
  workflow; no automatic private-memory upload is introduced.
