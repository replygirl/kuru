# Dependencies

## Blocked by

- [x] `project-memory-owner-foundation` — authenticated typed owner, generation and native lifecycle baseline *(archived 2026-09-22)*
- [x] `context-usage-accounting` — permanent branch and invocation-keyed ledger records *(archived 2026-09-16)*

## Soft-blocked by

None.

## Downstream phase gates

The active `project-memory-service` change must not enable ordinary managed writes until this retained outcome contract is implemented. P28 adds the broader store advisory and concurrent mutation proof; P29 then scopes raw histories, and P30 removes the conversation-driver lease. This prerequisite itself does not enable concurrent conversation admission.
