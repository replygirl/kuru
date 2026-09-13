## Why

Durable peer notes are currently written under a private `/notes` namespace,
but the human-facing `/memory` and memory CLI inspection paths expose peer
conversation or commit metadata instead. A person needs to read a selected
peer's durable notes without starting a provider, constructing a harness, or
mistaking those notes for the peer conversation.

This read path must preserve the runtime's current identity and mode rules. It
must make an incomplete bounded result explicit, and it must retain archived
identities that remain present in the current topology without inventing
cross-mode identity lookup.

## What Changes

- Add a provider-free runtime notes-read projection over the existing live
  `/notes` namespaces. The projection resolves the active mode, applies the
  current exact-ID-or-active-resolution rules, returns chronological notes,
  and reports the requested limit and whether older notes were omitted.
- Add `kuru memory notes <IDENTITY> --limit N`, defaulting to 100 and accepting
  1 through 1000, using an existing live Dolt store's read-only inspection path.
  A notes-only invocation does not trigger legacy import or seed missing topology.
- Add TUI `/notes ID`, using the same runtime projection with a requested limit
  of 100. Keep `/memory ID` as the existing peer-conversation command.
- Add isolated runtime, CLI, and TUI coverage for separate conversation/note
  content, archived exact identities, unknown identities, N+1 truncation and
  chronology, provider-free existing-store inspection, fresh-store
  non-creation, and typed TUI dispatch. Native Windows execution remains a
  required CI check and is recorded honestly when unrun locally.

## Capabilities

### New Capabilities
- `durable-notes-inspection`: provider-free human reading of bounded durable
  notes for a current-mode peer or relationship identity.

### Modified Capabilities

<!-- None. -->

## Impact

- `packages/kuru-runtime/src/engine.rs` gains a public, read-only notes view
  and identity/namespace reader; it does not construct or save a `Harness`.
- `apps/kuru-tui/src/cli.rs` adds the `memory notes` subcommand and provider-free
  dispatch through its existing read-only store path.
- `apps/kuru-tui/src/ui.rs` adds `/notes`; existing `/memory` behavior remains
  unchanged.
- Runtime and TUI/CLI isolated fixture tests gain coverage. `kuru-memory`, the
  store schema, migrations, prompt construction, provider routing, tool
  authority, retention, deletion, export, historical revisions, and candidate
  CLI access remain unchanged.
- Update the usage/memory documentation and curated command/memory references
  with the notes-versus-conversation distinction, mode/identity selection,
  result limits, and explicit truncation metadata.

## Surfaces

- [x] interactive — CLI and TUI notes inspection output
- [ ] deploy — no deployment topology or secret change
- [ ] integration — no external contract or dependency change
- [ ] agent-behavior — notes are never added to model prompts or tool results
