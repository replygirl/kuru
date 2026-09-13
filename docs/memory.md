# Memory storage

Kuru uses full Dolt for local, versioned memory. Each canonical project directory
has a separate database and revision history. Parts and relationships retain their
private namespaces inside that database. The SQL server binds to loopback with
generated local credentials and runs only while its owning Kuru process needs it.

```sh
kuru memory status
kuru memory history
kuru memory notes ID --limit 100
kuru memory forget ID --note SEQUENCE
```

Status identifies the current project store, branch and revision. History lists
committed memory updates. These commands do not expose database credentials.
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

## Runtime and offline use

Every Kuru executable includes the pinned native Dolt archive and its license
notices. First memory use verifies and extracts those bytes locally; it needs no
network, compiler or separate engine installation. The cache is `tools/dolt`
inside the Kuru data directory; `memory.cache_dir` selects another location.
Subsequent runs verify and reuse the extracted engine. Corrupt existing caches
fail explicitly and remain preserved for inspection.

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

The SQL schema version is independent from the format-1 `ready.json` activation
record, the database identity record, and the supervisor protocol. An old dream
candidate stays on its recorded historical schema and remains stale if `main`
has since advanced; it is never silently rewritten by an upgrade.

## Dream revisions and undo

Dreams use an isolated candidate branch. Their notes, histories, reports and
topology changes become active together after validation. A candidate based on an
outdated live revision cannot overwrite newer conversations. Undo records a new
revision restoring prior membership, while preserving later chats and preferences.

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

Kuru releases its owned sidecar when its lifetime pipe closes, including after a
writer crash. If startup reports a held lifecycle lock, wait for the owner to
finish. An unknown PID or occupied port never authorizes automatic termination.
Preserve a failed store and its logs before investigating; do not remove a held
lockfile or replace the directory while a process is using it.

Inspection commands can read an active writer's store. When an inspection starts
the server itself, a new writer waits for that command to finish before taking
ownership. Normal command exit waits for owned database cleanup, including when
the command reports an error. Migration and recovery also hold the lifecycle lock
through directory activation, so an active database cannot be moved underneath
another process.
