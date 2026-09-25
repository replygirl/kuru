## Context

P29 supplies two related reads. `session_history_window` selects the newest rows but removes their durable sequences and counts the whole session. `session_source_snapshot` preserves sequences and source provenance but selects the oldest forward page after a cursor and caps it at 1,024 rows. Neither can yield a checked newest suffix after a summary cursor when more than one page remains.

## Goals / Non-Goals

**Goals:**

- Give request assembly one bounded read for the newest exact sequenced suffix after a durable cursor.
- Preserve local, managed, live and candidate parity with explicit view and revision provenance.
- Reuse P29's row and serialized-byte bounds.

**Non-Goals:**

- Replace the oldest-forward source snapshot used to construct the next compaction input.
- Add paging, summary mutation, export access or context-selection policy to memory.

## Decisions

### Add a distinct cursor-relative newest-window DTO

The query accepts namespace, session, exclusive sequence and limit. Its result repeats those selectors, adds the pinned view and captured revision, reports the exact eligible-row count, and returns `SequencedMessage` rows. Extending `HistoryWindow` was rejected because its unsequenced message shape and whole-session count are existing contracts; changing them would broaden unrelated callers.

### Select newest rows in one checked view and return ascending order

Under the view's existing mutation guard, the store captures the revision and exact eligible count, streams rows in descending sequence order, stops at the caller's row limit or the shared 32 MiB serialized-row budget, and reverses the retained rows. Repeated forward snapshot paging was rejected because it is unbounded and each managed call may observe a different revision.

### Carry one read-only managed operation

The facade validates before selecting its local or remote backend. Managed service dispatch performs the same owner-side validation and returns one typed value; the operation has no logical receipt and cannot clear or create a mutation fence. Adding a request and response variant advances the protocol minor version so an old owner cannot decode the new operation.

## Risks / Trade-offs

- [Counting every eligible row can cost more than returning the suffix] → Keep the query scoped by the existing namespace/session index and bounded ordinary read deadlines; the exact count is required to distinguish omission from absence.
- [A large row can consume most of the response budget] → Reuse the 32 MiB aggregate bound and require an individually valid first row to fit, preserving at least one-row progress for positive limits.
- [The summary cursor may advance in another call] → The caller supplies the cursor it actually selected; the result binds that cursor and its own current view/revision without inferring a newer summary.
