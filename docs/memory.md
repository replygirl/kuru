# Memory storage

Kuru uses full Dolt for local, versioned memory. Each canonical project directory
has a separate database and revision history. Parts and relationships retain their
private namespaces inside that database. The SQL server binds to loopback with
generated local credentials. A private per-project memory service owns Dolt,
allows checked Kuru clients to attach, and remains available for a bounded idle
interval after the last attachment closes.

```sh
kuru memory status
kuru memory history
kuru memory candidates --limit 16
kuru memory candidate-status BRANCH
kuru memory candidate-abandon BRANCH --base BASE --head HEAD
kuru memory notes ID --limit 100
kuru memory forget ID --note SEQUENCE
kuru memory purge --yes
kuru memory export --format json --output committed-memory.json
```

Status identifies the current project store, branch and revision. History lists
committed memory updates in Dolt graph order, newest first; timestamps do not
decide the order. These commands do not expose database credentials.
`/memory-status` and `/memory-history` provide revision inspection in the TUI.
`/memory NAME_OR_ID` continues to inspect an identity's conversation. `/notes
NAME_OR_ID` reads that identity's separate durable notes. Both use the selected
mode; an exact retained part or relationship ID can be inspected after it is
inactive, while names and roles resolve only among active identities. Notes are
the newest requested messages in chronological order, with `requested_limit`
and `truncated` in the result. `kuru memory notes ID --limit N` accepts N from
1 through 1000 and defaults to 100. It reads an existing current Dolt store and
does not import legacy SQLite data or create memory for a fresh project.

Each returned note includes its stable `sequence` and stored `role`, including
notes written by dreaming. `kuru memory forget ID --note SEQUENCE` removes that
one row from the selected identity's active notes namespace and records a new
Dolt revision. It does not remove conversations, other notes, or older revisions;
it is not secure erasure and does not provide a history-recovery command.

Dream candidate writes remain private until promotion. `kuru memory candidates`
and `/memory-candidates` list bounded pages of retained exact refs; the returned
cursor is opaque. Candidate status distinguishes an unchanged open ref, a ref
whose live base moved, an uncertain transition, and a resolved or missing ref.
Missing reports its operation outcome as unproved: closing or reopening a client
does not settle it. Explicit abandonment requires the exact inspected branch,
base, and head, settles any retained typed outcome, and rechecks all three under
the owner before transition. Changed, active, historical, ambiguous, or
uncertain refs remain intact. There is no public candidate promotion, merge, or
automatic replay/abandonment command.

`kuru memory purge --yes` is the explicit destructive control for one canonical
project. It removes that project's managed current Dolt store, revision history,
and recognised managed recovery trees, then prevents that project from being
automatically re-imported from a shared legacy SQLite source. It refuses an
active owner and never kills it. It retains original/shared legacy SQLite inputs
and migration snapshots, user exports and backups, other projects, engine cache,
and stable lock files. After memory removal, it removes that project's bounded
diagnostics ring. If an earlier purge records an incomplete operation, rerun the
same `kuru memory purge --yes` command: it removes only the recorded remaining
identities. It is not secure erasure and does not rewrite copies outside Kuru's
managed project store.

`kuru memory export` writes every application message and state record from one
captured, committed `main` revision. It uses JSON by default; `--format markdown`
renders the same records as JSON fenced blocks. Without `--output PATH`, JSON is
written to standard output. With `--output PATH`, Kuru completes a private staged
file before publishing a new destination and refuses to overwrite an existing
file. The manifest records the project scope, revision, schema version, counts,
and explicit exclusions for previous revisions, candidate branches, uncommitted
rows, operations, and schema tables. Export never starts a provider, imports legacy SQLite data,
or creates a fresh memory store. It is a current committed snapshot, not a
historical-revision browser or a secure-erasure/archive facility.

The operational usage ledger is separate from the exported live-memory snapshot
and is not included in `memory export`. It belongs to the same managed project
store, so closing or purging that store also covers its ledger. Dream promotion,
abandonment and undo do not remove observed usage; `/cost` reads session usage
separately from conversation and note history.

## Runtime and offline use

Every Kuru executable includes the pinned native Dolt archive and its license
notices. First memory use verifies and extracts those bytes locally; it needs no
network, compiler or separate engine installation. The cache is `tools/dolt`
inside the Kuru data directory; `memory.cache_dir` selects another location.
Subsequent runs read every cached engine and license byte for its pinned digest,
revalidate the checked names and identities, run the exact-version probe and then
reuse the engine. Independent warm opens perform those checks concurrently; the
exclusive installation lock is reserved for missing-cache extraction and atomic
publication. Corrupt existing caches fail before execution and remain preserved
for inspection.

Before an application command opens memory, Kuru may print bounded progress on
standard error for the current work: waiting for private ownership, checking or
extracting the verified runtime, preparing and opening the database, then ready.
These messages do not estimate time or prove a stage succeeded; the command's
ordinary result remains authoritative. JSON and other command output stay on
standard output, and library callers do not receive progress messages.

Writable runtime commands attach to the private project memory service while
retaining the existing one-conversation driver lease. This phase does not admit
simultaneous conversations. Attachments do not own the service process and an
inspection handle cannot stop it. When the last attachment closes, the service
waits through its bounded idle interval, drains accepted work, reaps its exact
Dolt child, and only then releases lifecycle authority. A later command during
that interval can reuse the same checked service generation.

After the first conversation/runtime command (`kuru run`, `kuru dream`,
`kuru undo-dream`, the TUI, or `kuru serve`) opens a writable project store, the
application shows one informational local notice. It names the escaped managed directory and points to
`kuru memory notes ID`, `kuru memory export --format json --output PATH`,
`kuru memory forget ID --note SEQUENCE`, and `kuru memory purge --help`. The
notice records only that this project/version was displayed, after stderr is
flushed or the TUI frame is complete. It is not sent to a provider or added to a
peer conversation. Memory and chat do not expire automatically; forgetting a
selected active note retains prior Dolt revisions.

`memory.offline` remains accepted for configuration compatibility; bundled engine
provisioning always works offline. An explicit `memory.dolt_binary` is an optional
development override and must report the supported exact version. The authoritative
engine and platform pins live in `packages/kuru-memory/support/dolt-assets.json`.
Provider access is configured independently; `--provider demo` needs no inference
service. Build-time preparation is described in
[development](development.md#bundled-engine-build-inputs).

Fresh `--help`, `--version`, `config` and `update` do not provision a memory engine.
Reading saved project preferences requires an existing store. See
[configuration](configuration.md) for data-directory and startup options.

## Existing SQLite data

Close older Kuru sessions before upgrading. The first mutable open imports the
current project's data from `memory.sqlite3`. Kuru preserves the original and a
consistent full snapshot under `memory/legacy/`, including committed WAL data.
It compares imported rows before activating the new Dolt store. Other projects
remain in the original and snapshot until opened and imported in their own scope.

The new database lives under `memory/<canonical-project-hash>/`. Once activated,
Kuru reads and writes Dolt. Keep the original and snapshots until you have checked
all projects you intend to retain. Do not run an older SQLite-writing Kuru against
the same data directory after migration; its new writes will not update Dolt.

If migration fails, the original remains preserved. A staging directory is never
treated as the active store. A completed import can be resumed after validation;
partial or superseded imports are stopped and preserved under `memory/interrupted`
before retry. Unknown data fails explicitly. Correct the reported error and retry;
do not delete the source or bypass identity checks to force an import.

Memory-owned directories must remain private. On macOS and Linux, if Kuru
rejects a real directory owned by the current user because it grants group or
other access, it names that exact directory and asks you to restrict it to mode
0700 before retrying. This applies to project data, managed Dolt cache and
version directories, cold probes, installation destinations, and private server
directories, whether or not legacy SQLite is present. Kuru never changes the
mode automatically and does not offer that remedy for a link or foreign-owned
path. On Windows, correct the project data directory's owner-only access using
native file security settings and retry; managed runtime and server directories
retain their native filesystem diagnostics rather than receiving a Unix mode
instruction. Kuru leaves the rejected directory and its permissions unchanged;
correct the reported exact path before retrying.

## Schema upgrades

Writable opens apply compatible Dolt schema upgrades in order before making a
store available. Each step is built on an isolated internal branch and reaches
`main` only through a checked fast-forward after its committed receipt and
schema are validated. Failed or interrupted attempts remain preserved for
inspection; Kuru does not reset or delete them during startup. A read-only open
reports an older supported schema without changing it, and an unknown future or
inconsistent schema fails without modifying the store.

Startup also stops when retained migration attempts are ambiguous or exceed its
bounded inventory. It preserves those branches and reports the condition for
recovery rather than deleting or resetting history.

Schema 3 adds an explicit message-content format. Existing rows retain their
exact strings under `text-v1`, including strings that look like JSON. Typed
message writes use `typed-v1`, which stores an ordered block payload separately
from the row's role. Raw note/text appends remain text records. Readers use each
view's schema and format; malformed or unknown formats fail explicitly rather
than fall back to text. Supported old revisions and candidate branches remain
readable without rewriting them. Typed writes require an upgraded view.

Memory exports include the content format and use export format version 2.
Consumers must inspect that version and the per-message discriminator rather
than assume every content string is ordinary prose. JSON preserves the stored
payload, and Markdown identifies structured records. This does not add an
export-import command or rewrite existing user exports.

Schema 4 retains compact indexed operation receipts across later writes for
exact internal write-outcome checks. Existing
schema-1-through-3 candidate branches keep their historical schema and remain
inspectable; they are not rewritten or promoted across an upgraded main. An
older Kuru binary that does not understand schema 4 refuses the upgraded store.
Replacing the executable with an older release does not downgrade memory; use
a compatible Kuru to reopen it. Original legacy SQLite data remains intact.

The SQL schema version is independent from the format-1 `ready.json` activation
record, the database identity record, and the supervisor protocol. An old dream
candidate stays on its recorded historical schema and remains stale if `main`
has since advanced; it is never silently rewritten by an upgrade.

## Dream revisions and undo

Dreams use an isolated candidate branch. Their notes, histories, reports and
topology changes become active together after validation. A candidate based on an
outdated live revision cannot overwrite newer conversations. Undo records a new
revision restoring prior membership, while preserving later chats and preferences.

Current writable branches retain compact internal operation receipts for exact
lost-reply reconciliation. Historical schema-1-through-3 branches keep their
older receipt shape. Kuru reclaims a dream candidate only after its promotion
or explicit abandonment is durably resolved. Unresolved candidates,
conversations, notes and reachable Dolt revisions do not expire automatically.
The bundled engine performs bounded, growth-triggered storage maintenance while
Kuru owns it, but retained history can continue to grow. This maintenance is not
secure erasure. If a private install stage cannot be removed after the engine is
published and verified, Kuru keeps that stage with a receipt and collects it on a
later open; leftover stages are counted, and a sweep that leaves at least a small
cap of them behind is reported to diagnostics. No stage whose removal is still
uncertain is deleted. If even the receipt cannot be written, the startup notice
and diagnostics say so: that stage is never collected automatically and has to be
removed by hand.

## Backup and recovery

Revision history is stored on the same disk as the database. To make a local
backup, close all Kuru processes using the data directory, allow their supervised
Dolt processes to finish, and copy the complete data directory to your backup
location. Include `memory/`, its project metadata and any preserved legacy files.
Do not copy or replace a live `.dolt` directory.

Restore into a separate data directory and open it with `--data-dir`. Keep the
original backup until you have checked session history and memory status. Project
identity depends on the canonical workspace path; restoring the data directory
does not remap a project to a different path.

Kuru's private service releases its owned sidecar after its lifetime pipe closes,
including after a service crash, and a successor waits for exact reap before
election. If startup reports a held lifecycle lock, wait for the owner to finish.
An unknown PID or occupied port never authorizes automatic termination.
Preserve a failed store and its logs before investigating; do not remove a held
lockfile or replace the directory while a process is using it.

Inspection commands attach read-only to an active memory service. Without one,
they use an explicitly local read-only open and never elect an owner. Normal
runtime command exit awaits attachment cleanup, including when the command
reports an error; the service may remain warm through its idle interval.
Migration, recovery, purge, and other maintenance use explicit quiescence gates
and hold the lifecycle lock through directory activation, so an active database
cannot be moved underneath another process.

Within Kuru, dropping or explicitly closing a managed memory view releases that
client attachment; it never abandons a candidate or kills the shared owner. If a
write reply is interrupted, Kuru checks the durable typed receipt before
proceeding; reconnect alone never turns uncertainty into success or replays the
write.
