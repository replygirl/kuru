# Memory storage

Kuru uses full Dolt for local, versioned memory. Each canonical project directory
has a separate database and revision history. Parts and relationships retain their
private namespaces inside that database. The SQL server binds to loopback with
generated local credentials. A private per-project memory service owns Dolt,
allows checked Kuru clients to attach, and stops once no client is attached and
accepted work has settled.

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

`kuru memory export` writes every application message, state, context-summary
and context-cursor record from one captured, committed `main` revision. It uses
JSON by default; `--format markdown`
renders the same records as JSON fenced blocks. Without `--output PATH`, JSON is
written to standard output. With `--output PATH`, Kuru completes a private staged
file before publishing a new destination and refuses to overwrite an existing
file. The manifest records the project scope, revision, schema version, counts,
and explicit exclusions for previous revisions, candidate branches, uncommitted
rows, operations, and schema tables. Export never starts a provider, imports legacy SQLite data,
or creates a fresh memory store. It is a current committed snapshot, not a
historical-revision browser or a secure-erasure/archive facility.

This is a full-project export for its invoking owner, so it can include
producer-private state such as settled provider reasoning summaries. Treat the
output as private project data. Conversation transcripts, peer context, fork
presentation, and session export do not project those records. During an active
selected-speaker turn, the existing TUI may show a bounded transient text-only
reasoning preview; it carries no provider coordinates, clears with the turn,
and is never a durable record. The producing actor can use private state only
through a policy-selected replay or compaction path; Kuru never injects it into
every request.

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
Installation runs the freshly extracted engine once to confirm its exact version
before activating it. Subsequent runs read every cached engine and license byte
for its pinned digest, revalidate the checked names and identities and then
reuse the engine without running it again to check its version: the cache
directory is named for that pinned version, so a matching digest identifies the
engine installation already checked. An intact cached engine that nonetheless
cannot run on this system is reported when its database server fails to start.
Independent opens perform those checks concurrently; the exclusive installation
lock is reserved for missing-cache extraction and atomic publication. Corrupt
existing caches fail before execution and remain preserved for inspection.

While an application command opens memory, Kuru shows one plain sentence about
what it is doing at that moment, and nothing else: no title, no label, no
elapsed time and no line when memory is ready. These five sentences are the
whole set; each ends with an ellipsis.

- `Opening this project's memory…` is true for the whole open and is shown
  first.
- `Getting Kuru's memory ready on this computer…` while Kuru unpacks or checks
  its bundled database engine for the first time on this computer, or waits for
  another copy of Kuru that is doing so.
- `Creating this project's memory…` while the project has no memory yet and this
  open creates it, including finishing an import or an interrupted setup.
- `Upgrading this project's memory…` while an existing project's memory is
  upgraded to this version of Kuru.
- `Waiting for another copy of Kuru to finish with this project's memory…` while
  another Kuru process holds the project's memory, often the previous command's
  service while it shuts down.

The sentence follows the work being done, not a count of internal steps: stages
that begin nothing a person would recognise keep the current sentence, and so
does any stage added later. A stage shorter than one check may never be shown:
while a new memory service starts Kuru checks its progress about every 10 ms,
and while it waits for another copy of Kuru, every 100 ms. On a terminal the
sentence is rewritten in place on one line, shortened with its ellipsis kept
when the terminal is narrow, and erased when memory is ready, so nothing
remains. When standard error is not a terminal, each new sentence is written
once, whole, on its own line. An interactive session whose
standard error is redirected but whose standard output is a terminal still shows
the sentence on that terminal before the interface takes the screen; it is
erased before the interface opens. That is the only case in which it goes
through standard output; command output, including JSON, never receives it, and
library callers receive nothing.
The sentence does not estimate time and does not prove a stage succeeded; the
command's ordinary result remains authoritative. The sentence cannot fail or
delay the open: the owner publishes what it is doing in a small activity record
that grants no authority, is never read to elect, attach, recover or retire, and
is removed when the owner closes. If the record cannot be written the command
shows only the opening sentence. When an inspection command's own open installs
the engine and cannot remove leftover setup files, it prints one notice on a
line of its own after the sentence is erased and memory is ready, "Kuru could
not remove some leftover setup files; it will try again on a later start." when
the stage is receipted, otherwise the not-retried notice quoted below. A service
started on behalf of a command has no terminal and reports none.

Writable runtime commands attach to the private project memory service while
retaining the existing one-conversation driver lease. This phase does not admit
simultaneous conversations. Attachments do not own the service process and an
inspection handle cannot stop it. When the last attachment closes and accepted
work has drained, the service stops accepting, retires its endpoint, reaps its
exact Dolt child and only then releases lifecycle authority. There is no idle
interval: a later command starts a new service after that shutdown completes, so
each command that follows another opens the existing project again instead of
attaching to a running service.

A service that a command started waits for that command to attach before it may
retire, so another client that attaches and detaches first does not end it. If
the command never attaches, for example because it died first, the service keeps
serving other clients and exits at its first empty moment once
`memory.startup_timeout_secs` has passed since it published its endpoint, with a
warning in its log. A command that starts while the previous service is still
shutting down waits for that shutdown, shows the waiting sentence while it does,
and then starts or attaches to a successor, within the same timeout; meeting a
retiring service is not an error in itself. If the shutdown outlasts the timeout, the error says the previous
service was still shutting down.

A writable session keeps at least one attachment across a cancelled or failed
memory call. Kuru does not resend the request; the reply is still recovered from
its durable receipt. When a call on the session's main connection is cancelled or
fails, the client keeps that connection open, unused, and opens a replacement to
the same service generation. It closes the old connection only after the
replacement has completed its handshake, so pressing Esc during a memory call
does not stop the service under a running session. A replacement never elects or
starts a service. Read-only commands hold no such connection: a read-only call
that was cancelled after the service retired reports "the memory service ended
while this read-only command was disconnected; run the command again". Read-only
commands never start a service, and a writable command's service retires as soon
as the command exits.

This covers the cases where the client can keep the service. Three cases it does
not cover fail the next call with a truthful error instead of hiding the fault:
the service closes a live client's only connection on its own, for example after
a malformed request frame; the replacement connection is refused (the service is
at its attachment limit, or is settling an explicit candidate abandonment) and
the service then ends the abandoned connection after its 35-second operation
timeout; and a call dropped outside an async runtime, which opens no
replacement until the next call. An explicit candidate abandonment issued from
the same session in the moments after a cancelled call can be refused as active
until the cancelled request finishes; running it again succeeds.

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

Memory exports include the content format and use export format version 3.
Consumers must inspect that version, each message's optional session identity,
and the per-message discriminator rather than assume every content string is
ordinary prose. JSON preserves unattributed legacy messages with a null session
identity and preserves strict context summaries and cursors as separate record
kinds. Markdown identifies the same structured records. This does not add an
export-import command or rewrite existing user exports.

Schema 4 retains compact indexed operation receipts across later writes for
exact internal write-outcome checks. Existing
schema-1-through-4 candidate branches keep their historical schema and remain
inspectable; they are not rewritten or promoted across an upgraded main. An
older Kuru binary that does not understand schema 4 refuses the upgraded store.
Replacing the executable with an older release does not downgrade memory; use
a compatible Kuru to reopen it. Original legacy SQLite data remains intact.

Schema 5 adds nullable physical session provenance to the global message
sequence and strict context-summary/cursor tables. New conversation transcript
and actor-history writes carry their session identity. Existing and imported
rows remain unattributed rather than being assigned to a guessed session; they
remain available to explicit inspection, export and recovery. The ordinary
conversation-driver lease remains in force while later Phase 2 work integrates
concurrent admission. Context assembly already admits bounded same-actor current
summaries from other sessions only through the mode's own-history visibility and
memory namespace. It keeps the current session's cursor-selected summary
separate, omits older shared summaries whole when the model budget requires it,
and never substitutes another session's raw rows or private reasoning sidecars.

Schema 8 records each published schema step. Every opening checks the retained
upgrade branches, and before schema 8 it checked each one in full: its committed
schema, receipt, parent and place in history. From schema 8, the commit that
publishes a step also records the step's branch, the commit it was built on,
its receipt and its definition in `kuru_migration_publications`. Later opens
check a recorded branch against that record, from `main`'s own history, instead
of checking it in full. Every check still has to pass: the branch has no
uncommitted changes, its head's only parent is the recorded base, both are in
`main`'s history, and the head carries the recorded schema and receipt. A
recorded branch that has since been deleted is accepted while its recorded base
is still in that history. A branch without a record, such as a failed attempt,
is still checked in full. If a record disagrees with the store in any way, the
open fails without changing anything and without falling back to the full check.
No record is ever inferred from a branch's name.

The first writable open after updating to a release with schema 8 upgrades an
existing project once. That open checks the retained branches in full one last
time, and the schema-8 step records every branch that check accepted, in the
same commit. Like any schema upgrade, this open starts the database twice: once
to upgrade, and once to reopen the upgraded store. Until that writable open
happens, a read-only command on the project, such as inspection or export,
fails with `memory schema version 7 requires writable upgrade to 8`.
This is the same refusal every pending schema step produces, and it changes
nothing. Projects created from the store template after the update start at
schema 8 with their records and never need this upgrade.

The SQL schema version is independent from the format-1 `ready.json` activation
record, the database identity record, and the supervisor protocol. An old dream
candidate stays on its recorded historical schema and remains stale if `main`
has since advanced; it is never silently rewritten by an upgrade.

## New projects and the store template

New projects are created from a per-machine store template: a copy of a store
that has already run every schema step. With the template for this release
already on the machine, a new project opens with two database starts: one that
adopts the copy, validates it and marks it ready in its staging directory, and
the ordinary start at the project's own path. No schema step runs for it.

The first new project on a machine, and the first after an update that changes
the template's key (a new schema step, engine version or store-creation
statement), builds the template first: every schema step runs once, on one
database start, and the project is then copied from the result, in three
starts. That first open does about as much work as building the store directly;
every later new project skips the schema steps. The template is shared by every
data directory that uses the same engine cache.

While a new project is created, whether it is copied from the template, the
template is built first, or the project is built directly, Kuru shows
`Creating this project's memory…` (after `Getting Kuru's memory ready on this
computer…` while a first launch unpacks the engine). Building the template is
part of creating the project, so the sentence stays the same throughout and
no other sentence is shown for it.

A new project is built directly instead, running every schema step in its own
staging directory (four database starts), when:

- it imports legacy SQLite data (the import runs before the schema steps);
- `memory.dolt_binary` names a development engine, because the template's key
  binds the bundled engine;
- another process is building the template or moving a damaged one aside at
  that moment: a new project never waits for another one's build;
- the template's directory or lock file cannot be opened, locked or verified,
  which is logged as a warning in the memory service log; or
- a template this open did not build fails a check while it is copied. A
  partial copy is preserved under `memory/interrupted/` without starting a
  database. A template whose own bytes failed the check is moved aside, and the
  next new project builds a fresh one; an I/O error leaves it in place.

None of these is an error: the project opens as before, only more slowly.

The one damaged-template case you can see is a copy that the project's own
database refuses after copying it: adoption finds an identity other than the
template's placeholder, or the shape check finds anything but the expected
branches, commits, schema and placeholder identity. That open fails with a
template verdict naming what differed, nothing appears at the project's path,
and the template is moved aside. Any other failure of that database start (a
crash, a deadline, a lost reply, or a copy made for a different Kuru build)
fails the open the same way but leaves the template in place. Either way the
unready copy is preserved under `memory/interrupted/`: at once when the
failure came after its database was serving (the shape check, validation or
marking it ready), and otherwise by the next open of the project, before it
starts anything and without starting the copy's database. That next open then
creates the project again: from the template when it is still in place, or by
building a new one when it was moved aside.

A failure of the template build that a first launch runs (its database, its
own checks, or saving its result), or of copying the project from the template
it just built, also fails that open, with that error; opening again tries
again. A partial copy is preserved under `memory/interrupted/` without starting
a database. Only a copy that finds the new template's own bytes differ from its
manifest moves that template aside, and the next new project builds it again;
any other such failure moves no template aside.
Kuru never retries inside the same open, so the schema steps never run twice
in one open.

The template lives beside the engine it was built with, in the engine cache
(`tools/dolt` in the data directory, or `memory.cache_dir`):
`<cache>/<engine version>/templates/<key>/`. It is the `data/` directory of a
store that Kuru built once, under a fixed placeholder identity, by running the
real schema steps on one database start. It holds the schema, the migration
receipts and that placeholder identity only: no project data, no identity
record, no credentials, no database configuration or home directory. Every
project that shares the cache can therefore share it, and purging a project
(`kuru memory purge`) does not touch it.

The key in its name covers exactly what decides a fresh store's bytes: the
template format, the pinned engine version and this platform's engine digest,
the schema versions and every schema step, the format of the migration
publication records, and the statements and settings that create a store. A
release that changes any of them uses a new key and builds a new template once;
old templates stay, as old engine versions do. The release that adds schema 8
is one of these: the first new project after updating builds the template
once, running every schema step once.
Nothing removes a template or its lock file during ordinary use.

A template is built only by the holder of its key's exclusive lock, and is
published only after its database validates, its shape matches exactly the
schema, receipts and placeholder identity the schema steps produce, and a scan
of its files finds none of the build directory's path, the build's database
credentials or the host name. The shape check also reads every commit on
`main` and on the usage branch: the engine's own first commit under its fixed
system account, then Kuru's commits under Kuru's fixed author with the
messages the schema steps write. That scan sees literal bytes only (the engine
compresses its storage), so the shape check, which reads the database itself,
is the guarantee and the scan an extra precaution; it is what rules out a
recorded host or operating-system user name. The scan skips host names
shorter than four characters, and does not look for the user name at all,
because such short sequences occur by chance in compressed files and would
refuse sound builds. A build whose scan finds anything is refused and its
capture is removed at once. Copying a template checks
every file's size and digest against its manifest and refuses links. A
template is moved aside as `.rejected-<key>-<id>` only when a check of its own
bytes fails; an engine, I/O, lock or deadline failure leaves it in place. No
caller ever waits for another process's build: a busy template means building
the store without one.

A build's own working store (`.build-<key>-<id>/`, which holds the build's
private identity record and its database credentials) is removed once its
template is captured. If that removal fails, or a builder stops before it,
the next holder of the key's exclusive lock removes it, without waiting for an
engine that has not yet stopped: the next build, or the next quarantine of
that key's template. Until then it stays in the cache directory, which is
private to your user.

A template directory Kuru cannot read, for example one whose permissions are
no longer private to your user, is an I/O failure rather than a verdict
against its bytes, so it is never moved aside. Each new project is then
built without it, with a warning, until the directory is fixed. To recover, delete `<cache>/<engine version>/templates/<key>/` while
no Kuru process is running; the next new project builds it again. On Windows
the same `templates/` directory also holds a small `lifecycles/` lease file per
build; like the key lock files, these are permanent and go only with the cache
directory.

## Template-born stores

A copied store records the template it came from in its private identity
record (`identity.json`, field `template`). A store created directly or by
import has no such field and keeps its identity record byte-for-byte. The
copy's first database start adopts it once: it checks that `main` and the
usage branch each still hold only the template's fixed placeholder identity
with clean working sets, then rewrites that row to the project's own identity
with one commit on the usage branch and one on `main`, and creates new
database credentials. Copies of one machine's template share only its schema
history (the schema initialization and migration commits under the
placeholder identity), never project data or credentials; each project's own
history starts at its adoption commits, and its initial revision is the `main`
adoption commit. Before the copy is marked ready, Kuru validates it and checks
that it holds nothing but the expected branches, commits, schema and identity.

A copy whose bytes differ from what this build expects is refused with a
distinct template verdict; an engine, SQL, I/O or deadline failure, or a copy
made for a different Kuru build, is an ordinary error. Either way the unready
copy is never placed at the project's active path and is never activated: a
failure after its database was serving preserves it under `memory/interrupted/`
at once, and one before leaves it in its staging directory for the next open.
If a crash or such a failure interrupts a copy before or during adoption, the
next open moves it under `memory/interrupted/` without starting its database
and creates the store afresh; a copy already marked ready is activated like
any completed stage. A copy is compared with the Kuru build only before
adoption, so an adopted store keeps opening under later releases. A release
without this support fails closed on a template-born store's identity record
rather than misreading it.

## Dream revisions and undo

Dreams use an isolated candidate branch. Their notes, histories, reports and
topology changes become active together after validation. A candidate based on an
outdated live revision cannot overwrite newer conversations. Undo records a new
revision restoring prior membership, while preserving later chats and preferences.

Current writable branches retain compact internal operation receipts for exact
lost-reply reconciliation. Historical schema-1-through-4 branches keep their
older receipt shape. Kuru reclaims a dream candidate only after its promotion
or explicit abandonment is durably resolved. Unresolved candidates,
conversations, notes and reachable Dolt revisions do not expire automatically.
The bundled engine performs bounded, growth-triggered storage maintenance while
Kuru owns it, but retained history can continue to grow. This maintenance is not
secure erasure. If a private install stage cannot be removed after the engine is
published and verified, Kuru keeps that stage with a receipt and collects it on a
later open; leftover stages are counted, and a sweep that leaves at least a small
cap of them behind is reported to diagnostics. An installation that stops before
publishing, after an error or a cancelled open, is handled the same way: a stage
it cannot remove is receipted as unpublished and reported to diagnostics, and the
receipt and diagnostic name the stage. An error the installation itself returns
names it too; a cancelled open returns none, and an internal failure such as a
crashed worker may not. Kuru records a retained stage before it releases the
installation lock, so another installer never finds an unrecorded one. A failed activation instead keeps its stage
deliberately as evidence and names it in the error. No stage whose removal is
still uncertain is deleted. If even the receipt cannot be written, diagnostics
say so, as does a notice on standard error when an inspection command's own open
published the engine: "Kuru could not remove some leftover setup files from the
tools folder of its data directory, and will not retry. Remove them by hand when
no copy of Kuru is running." (with `memory.cache_dir` set it names that folder
instead). That stage is never collected automatically and has to be removed by
hand.

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
they use an explicitly local read-only open and never elect an owner. An
inspection that meets a service that is still shutting down waits, within
`memory.startup_timeout_secs`, until that service has reaped its engine and
released its owner lock, then opens locally; it does not read through the
retiring service's engine. Normal
runtime command exit awaits attachment cleanup, including when the command
reports an error; its service retires once its last attachment closes.
Migration, recovery, purge, and other maintenance use explicit quiescence gates
and hold the lifecycle lock through directory activation, so an active database
cannot be moved underneath another process.

Within Kuru, dropping or explicitly closing a managed memory view releases that
client attachment; it never abandons a candidate or kills the shared owner. If a
write reply is interrupted, Kuru checks the durable typed receipt before
proceeding; reconnect alone never turns uncertainty into success or replays the
write. Kuru answers from the durable receipt as soon as it exists, and
otherwise waits for the interrupted write to finish, within the existing
operation deadline.
