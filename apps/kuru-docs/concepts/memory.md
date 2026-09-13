# Parts, relationships & memory

Kuru's persistent unit is a part. A part has a stable identity, a role, and its own history. It can send a message directly to another active peer; there is no model that supervises every other model call.

The runtime schedules those calls and enforces limits on rounds, tools, and concurrency. The provider performs inference. This keeps the structure of the pool separate from the model you choose.

## Memory has boundaries

| Scope               | What it holds                     | Who receives it                              |
| ------------------- | --------------------------------- | -------------------------------------------- |
| Shared conversation | Your prompts and public responses | Actors given that session context            |
| Part memory         | A part's private history          | That part                                    |
| Relationship memory | The history of a particular group | That relationship and its authorized members |

Joining a relationship does not merge the members' private memories. One part's private history is not silently concatenated into another part's prompt.

You can inspect identities with `/parts` and their stored conversation history
with `/memory NAME_OR_ID`. `/notes NAME_OR_ID` and `kuru memory notes ID` read
the separate durable notes namespace. The notes view returns chronological newest
notes plus its requested limit and `truncated` flag, so it does not imply a full
export. The human user can inspect private memory; the separation governs what
the actors receive.

## Relationships

Two to four unique active parts can form a **protection**, **polarization**, or **alliance** relationship. Parts may propose these relationships during a turn. You can also activate one explicitly:

```text
/parts
/relate alliance FIRST_ID,SECOND_ID
```

Use IDs from `/parts`. A relationship has an identity derived from its kind and membership, independent of member ordering. Its history survives after it stops speaking.

A relationship can become the voice you are talking to. Use `/focus NAME_OR_ID` to select a speaking identity and `/focus auto` to return to contextual selection. Names must be unambiguous; an ID is useful when several parts share a role.

## Project scope

Histories persist across sessions within their project and framework scope. Project identity comes from the canonical directory path: opening the same directory through a symlink uses the same project. Another directory has a separate scope.

Changing frameworks selects a different topology. It does not give the new pool unrestricted access to other identities' private histories. Jungian Collective memory is also project-scoped.

## Local storage

Kuru stores durable state in a separate Dolt database for each project:

```text
$XDG_DATA_HOME/kuru/memory/<canonical-project-hash>/
```

If `XDG_DATA_HOME` is unset, the data directory is `~/.local/share/kuru` on macOS/Linux or `$env:LOCALAPPDATA\kuru` on Windows, with `$env:USERPROFILE\AppData\Local\kuru` as its Windows fallback. `--data-dir` or `KURU_DATA_DIR` chooses a separate directory. Keep it outside the tool workspace and restrict access as you would any chat history.

Kuru includes its verified native Dolt engine and license notices in the executable. First memory use extracts them locally, including when offline; later runs verify and reuse the cache at `tools/dolt` inside the data directory. No separate engine installation or runtime download is needed. `memory.cache_dir` selects another extraction directory. Corrupt existing caches fail explicitly and remain preserved.

The authenticated SQL sidecar runs only while its owning Kuru process needs it. Existing SQLite data is imported from a consistent snapshot; the original and snapshot remain preserved.

Use `kuru memory status` to inspect the store and current revision, `kuru memory
history` to list committed changes, or `kuru memory notes ID --limit N` to read
one identity's current-mode notes from an existing live store. Exact retained
part and relationship IDs remain readable after inactivity; names and roles use
active identity resolution. Notes limits are 1–1000, defaulting to 100. Dream
candidates stay private until promotion. Undo adds a compensating revision and
preserves later conversations.

Notes output includes a stable sequence and stored role for each current row.
`kuru memory forget ID --note SEQUENCE` explicitly removes one selected active
notes row and commits that change as a new revision. It does not alter a
conversation, other notes, or earlier revisions. This is active-memory control,
not secure erasure, and Kuru does not expose a history-recovery command.

`kuru memory export` reads every application message and state row from one
captured committed `main` revision. The JSON default and Markdown option preserve
the same rows and a manifest with revision, schema and row counts. Previous
revisions, candidate branches, uncommitted rows, operations and schema tables are excluded. Export is
provider-free and refuses fresh stores, legacy import, and overwriting an output
path; it is not a history-rewrite, purge, or secure-erasure operation.

History follows the Dolt commit graph from the current revision toward older
ancestors, with a stable hash tie-break rather than wall-clock ordering. Kuru
reconciles an interrupted write against its durable receipt before it continues,
so it can distinguish no pending operation, a completed operation, and one that
did not commit.

An operating-system writer lock prevents two Kuru processes from overwriting the same project's topology. Read-only session listing remains available.

## Migration and backups

Close older Kuru sessions before the first launch with Dolt. Kuru imports the current project's rows from `memory.sqlite3`, verifies them, and preserves the original plus a complete snapshot under `memory/legacy/`. Other projects are imported when opened. After migration, older Kuru versions write only to the old SQLite store, so avoid using them with the same data directory.

An interrupted import can resume after validation. Partial imports are stopped and preserved under `memory/interrupted/`; a failed import never becomes the active store. Keep the original and snapshots until you have checked every project you want to retain.

Kuru also applies compatible database schema upgrades automatically when a
writable project opens. Each upgrade is prepared on an internal isolated branch
and is fast-forwarded only after its committed receipt and schema validate.
Interrupted attempts are retained for inspection instead of reset or deleted.
Read-only access reports an older supported schema without changing it; an
unknown or inconsistent schema stops safely. The database schema version is
separate from the format-1 activation record, database identity record and
supervisor protocol format, and older dream candidates remain historical rather
than being rewritten.

Startup also stops if retained migration attempts are ambiguous or exceed its
bounded inventory. It keeps that history and reports the condition for recovery
rather than deleting or resetting branches.

Revision history shares the database's disk. For a backup, close all Kuru processes using the data directory, let their database processes finish, then copy the entire data directory. Restore the copy into a separate location and open it with `--data-dir`. Keep the same canonical workspace path to retain the project identity. Do not copy a live `.dolt` directory or remove a held lockfile.

See [sessions and dreaming](./sessions) for resuming a transcript and changing the pool's membership.
