## Context

The current notes reader obtains `Message` values through `MemoryStore::history`,
which deliberately omits database sequences because conversation callers do not
need them. The store already serializes mutations, records an operation receipt,
commits with Dolt, and reconciles a lost acknowledgement. Those guarantees can
cover a selected active row without a schema change.

## Goals / Non-Goals

**Goals:**

- Carry stable current note sequences through the existing selected-mode notes
  projection and delete one validated current notes row atomically.
- Make the CLI result and user documentation clear that the operation retains
  historical revisions and unrelated text.

**Non-Goals:**

- A restore command, history rewrite, secure erasure, automatic expiry, export,
  project purge, a new note table, or a new permission model.

## Decisions

### Keep sequenced notes separate from general messages

`kuru-memory` will return a small sequenced note DTO containing `sequence`,
`role`, and `content`; `NotesView` will use it. Extending shared `Message` would
change every transcript and provider contract even though only the human notes
projection needs a stored identifier.

### Delete by exact validated notes namespace and sequence

The runtime derives the selected current-mode `/notes` namespace after existing
identity resolution, and the store deletes exactly the row with that namespace
and sequence. It does not filter by role: existing remembered rows and
dream-authored rows are both current notes. A missing row aborts the transaction
before an operation receipt or revision is committed. A generic row deletion API
was rejected because it could make transcript deletion reachable from an
otherwise notes-only control.

### Reuse the owned mutation receipt

The delete is a new `Mutation` variant and follows the existing lock,
operation-ledger, Dolt-commit, and uncertain-receipt reconciliation sequence.
Adding a restore table or a second recovery path was rejected: durable revision
history explains retention but does not imply a supported recovery UI.

### Keep the CLI provider-free and preserve early refusal

`forget` joins the existing notes-only absent-store check before legacy import,
directory creation, lease acquisition, provider construction, tools, or a fresh
store. Once an existing store is confirmed it opens a writable memory handle for
the one mutation. The result reports the resolved identity, sequence, and
retained-history fact.

## Risks / Trade-offs

- [Concurrent delete or stale sequence] → exact affected-row validation reports
  failure and leaves unrelated current rows unchanged.
- [Caller loses a successful commit reply] → existing operation receipt
  reconciliation determines whether the one deletion committed without replay.
- [Users infer broader erasure] → CLI result and curated docs say the active row
  is removed while prior revisions and unrelated text remain.

## Operational surface

The control is a local existing-store CLI mutation and a local TUI read
projection. It adds no listener, container, secret, provider connection, or
binary/architecture requirement; the existing native Dolt supervisor and its
per-project writer lease remain the operational boundary.
