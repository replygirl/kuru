## ADDED Requirements

### Requirement: Provider-free bounded durable notes view

The runtime SHALL provide a public read-only `NotesView` for the selected live
mode and identity. It SHALL contain the resolved `Mode`, canonical identity ID,
chronologically ordered durable note messages, the requested limit, and a
`truncated` boolean. The reader SHALL derive only the existing
`{scope}/{mode}/identity/{id}/notes` namespace and SHALL NOT construct or save a
`Harness`, start a provider, grant tools, construct prompts, or read a candidate
or historical revision view.
It SHALL reject an explicitly supplied candidate view and a selected mode with
no persisted topology without seeding built-in identities or creating state.

#### Scenario: Notes remain separate from peer conversation
- **WHEN** an isolated live store contains different markers in an identity
  conversation namespace and its `/notes` namespace
- **THEN** the notes view returns only the note marker and its preserved
  role/content, with neither marker introduced into provider input or a prompt

#### Scenario: Bounded result reports omitted older notes
- **WHEN** an identity has more notes than a valid requested limit
- **THEN** the reader obtains one additional bounded row, returns the newest
  requested number in chronological order, and sets `truncated` to true

#### Scenario: Complete bounded result remains explicit
- **WHEN** an identity has no more notes than a valid requested limit
- **THEN** the reader returns those notes in chronological order and sets
  `truncated` to false

### Requirement: Current-mode human identity resolution

The notes reader SHALL load topology only for the caller-selected current mode.
An exact ID of any part or relationship present in that topology SHALL resolve,
including an inactive archived part; a non-exact input SHALL use the existing
active routing resolution rules. It SHALL reject an unknown or ambiguous input
and SHALL NOT search other modes or invent an identity from an archived name or
role.

#### Scenario: Exact archived part remains readable
- **WHEN** the selected mode topology contains an inactive archived part with
  retained durable notes and the caller supplies its exact ID
- **THEN** the view resolves that ID and returns its notes without activating the
  part

#### Scenario: Unknown identity is rejected
- **WHEN** the caller supplies an ID, name, or role that does not resolve under
  the selected mode's existing rules
- **THEN** the reader returns an identity error and reads no other identity
  namespace

### Requirement: Human notes commands preserve read-only inspection

`kuru memory notes <IDENTITY> --limit N` SHALL accept only limits 1 through
1000 and default to 100. It SHALL select mode through the existing explicit
invocation and saved-preferences configuration rules, open only an existing
project store through the existing read-only inspection path, and serialize the
complete `NotesView`. It SHALL fail without creating state when the project has
no memory. The TUI `/notes ID` command SHALL dispatch through the same runtime
notes reader with requested limit 100. Existing `/memory ID` behavior SHALL
remain peer-conversation inspection.

#### Scenario: CLI reads an existing store without provider startup
- **WHEN** an isolated existing project store has durable notes and the CLI is
  configured with an unavailable provider route
- **THEN** `memory notes` returns the notes view without provider construction,
  tool authority, a new session, or a memory write

#### Scenario: CLI fresh inspection creates no memory
- **WHEN** `memory notes` targets a project with no existing store
- **THEN** it returns the existing no-memory error and leaves the configured
  data directory absent

#### Scenario: Legacy-only inspection remains read-only
- **WHEN** `memory notes` targets a legacy-only layout with no current Dolt project store
- **THEN** it reports no current memory, performs no import or directory activation,
  and preserves the original legacy data

#### Scenario: TUI command uses typed notes dispatch
- **WHEN** an isolated TUI runtime receives `/notes ID`
- **THEN** it renders the same bounded notes view for that ID and leaves the
  peer conversation command's result unchanged
