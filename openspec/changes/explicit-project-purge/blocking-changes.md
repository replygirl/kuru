# Dependencies

## Blocked by

None.

## Soft-blocked by

- [x] `ordered-dolt-migrations` — source-present current-schema and recovery validation retained by purge/open integration *(archived 2026-09-13)*
- [ ] `memory-export` — source-present committed-snapshot command and documentation boundary; exports remain outside purge ownership

## Siblings

`selected-note-forgetting` remains a revision-preserving `/notes` mutation.
This change never rewrites selected history, adds a restore UI, or removes
shared legacy SQLite input.
