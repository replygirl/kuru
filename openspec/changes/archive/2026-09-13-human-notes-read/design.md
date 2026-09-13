## Context

`MemoryStore` already reads a named namespace with a caller-supplied limit, and
the runtime already knows the live topology, exact archived-ID exception, and
active identity resolver. The current CLI opens an existing store read-only,
loads saved preferences, finalizes explicit invocation overrides, and returns
before provider construction for memory inspection. The TUI has a live
`Harness`, but its existing `/memory` projection is conversation-only.

## Goals / Non-Goals

**Goals:**

- Keep namespace construction and identity resolution in `kuru-runtime`.
- Return a typed, serializable answer whose selection limit and incompleteness
  are visible to CLI and TUI callers.
- Reuse the current read-only CLI lifecycle without creating a session or store.

**Non-Goals:**

- This is not a complete export, a raw-store browser, a prompt-filtering
  policy, a retention operation, or a candidate/historical revision reader.

## Decisions

### One runtime projection serves both adapters

Expose `NotesView` from `kuru-runtime` and a provider-free reader with the
shape `read_notes(memory, cwd, mode, identity, limit) -> NotesView`; a small
`Harness` convenience method may delegate to the same resolution/read helper
for the TUI. `NotesView` contains `mode`, canonical `identity`, `notes`,
`requested_limit`, and `truncated`.

The alternative is for the CLI and TUI each to concatenate the `/notes` key and
resolve identities. That would duplicate the archived-ID exception and let the
two surfaces drift from routing semantics.

### Read one extra row and retain the newest requested window

Validate a limit in the runtime boundary, request `limit + 1` from the existing
store, remove the oldest row only when that extra row exists, and mark
`truncated` then. `MemoryStore::history` already yields its selected newest
window in chronological order, so the retained answer preserves chronological
order without a memory-package API change.

The alternative is to return only `limit` rows. It cannot distinguish a
complete result from an omitted older note and would make an incomplete answer
look complete.

### Select mode before reading, without starting runtime machinery

The CLI uses an existing live Dolt store: open it read-only,
load project preferences, then finalize the parsed snapshot so explicit mode
selection remains authoritative. It passes the resulting `config.mode` to the
runtime reader and returns before provider/tool/harness setup. TUI uses its
already selected harness mode and the same reader semantics.

The current general CLI startup can import a legacy SQLite file even for an
inspection command. The new notes-only path MUST reject a project without an
existing Dolt store before that import/lease/directory-creation path. Existing
commands retain their legacy behavior. A notes request against a legacy-only
layout reports no current memory and leaves the original layout untouched.

The runtime reader requires a live view and a persisted topology for the
selected mode. It must not call `read_topology`'s built-in seeding fallback when
that topology is absent. Reject a candidate view using existing store status,
and report missing saved topology without inventing an identity or writing it.

The alternative is to construct a temporary `Harness` for inspection. Its
constructor saves state, syncs actors, needs a provider and tool host, and can
create a session, all of which violate inspection-only behavior.

## Operational surface

The feature adds a local CLI subcommand and TUI slash command only. It opens the
same private existing Dolt store as `memory status/history`, has no listener,
container, service, credential, provider, tool grant, network request, or new
binary/architecture requirement. Read errors use the existing command error and
owned-store cleanup path; JSON output is the serialized `NotesView` on stdout.
User documentation in `docs/usage.md`, `docs/memory.md`,
`apps/kuru-docs/reference/commands.md`, and `apps/kuru-docs/concepts/memory.md`
will explain both commands, current-mode identity selection, archived exact IDs,
and the bounded newest-note window with explicit truncation metadata.

## Risks / Trade-offs

- **[A message list can be mistaken for a full history]** → include the
  requested limit and `truncated` in every view and test exact-boundary cases.
- **[A part instruction is model-prompt material]** → notes reading does not
  serialize topology or instructions; `/parts` remains the existing explicit ID
  discovery surface.
- **[A concurrent live write can occur during inspection]** → this bounded
  current-view reader makes no cross-namespace snapshot claim and does not add a
  historical/candidate export surface.
