# Dependencies

## Blocked by

- [x] `session-scoped-memory-provenance` — supplies schema-v5 session/source checkpoints, pinned revisions and the typed managed-memory boundary *(archived 2026-09-22)*

## Soft-blocked by

None.

## Delivery ordering

The exact reviewed `project-memory-service` implementation is branch ancestry,
so this bounded storage prerequisite can use the real facade and receipt path.
That active change must still archive and merge before this dependent change is
delivered. `actor-context-compaction` consumes this API and is not an upstream
dependency.
