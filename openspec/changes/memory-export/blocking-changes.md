# Dependencies

## Blocked by

None.

## Soft-blocked by

- [ ] `ordered-dolt-migrations` — source-present version-dispatched historical-schema validation and committed receipts; native/integrated gates remain active
- [ ] `store-api-hazards` — source-present closed-view and deterministic revision inspection semantics; its active record still carries integrated gates

## Siblings

`human-notes-read` and selected-note controls remain separate narrowed views and
mutations. This export consumes neither runtime identity resolution nor their
command surface.
