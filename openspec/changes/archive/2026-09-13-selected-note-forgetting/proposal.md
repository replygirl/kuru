## Why

Kuru can read one identity's current durable notes, but the projection omits the
stored sequence that identifies a particular note. The only current deletion
primitive clears every message in a namespace, so a person cannot deliberately
remove one active note while preserving unrelated notes and conversations.

Versioned Dolt history deliberately retains prior revisions. The selected-note
control must therefore describe its effect truthfully instead of implying secure
erasure, automatic expiry, or removal of related text elsewhere.

## What Changes

- Include stable stored note sequence IDs in the existing provider-free notes
  view for the selected mode and resolved part or relationship identity.
- Add `kuru memory forget ID --note SEQUENCE` to remove exactly that current
  stored row in the validated `/notes` namespace, including existing `note` and
  `dream` roles, through the existing owned mutation and reconciliation path.
  It refuses absent stores before legacy import or state creation, and it does
  not start a provider, tool host, or conversation.
- State in CLI and curated memory documentation that forgetting adds a new
  revision: prior Dolt revisions and other transcripts or notes remain. There
  is no restore command, history rewrite, secure-erasure claim, automatic
  expiry, whole-project purge, or export in this change.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `versioned-memory`: expose selected current notes with stable IDs and allow
  explicit, single-note active-view forgetting with a retained-history
  disclosure.
- `public-documentation`: document the selected-note command and its exact
  retention boundary.

## Impact

- `kuru-memory` gains a sequenced note read and exact note deletion mutation;
  no schema, migration, or dependency changes.
- `kuru-runtime` projects note sequences and owns current-mode identity and
  namespace selection.
- The TUI CLI and curated memory/command references expose the command and
  truthful disclosure. Existing `/memory`, `/notes`, identity resolution, and
  provider routes retain their behavior.

## Surfaces

- [x] interactive — provider-free CLI and TUI notes projection
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
