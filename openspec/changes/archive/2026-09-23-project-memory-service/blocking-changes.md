# Dependencies

## Blocked by

- [x] `private-local-service-ipc` — checked Unix sockets and successive-client Windows named pipes *(archived 2026-09-21)*
- [x] `project-memory-owner-foundation` — internal owner election, authenticated typed transport and native lifecycle fixtures *(archived 2026-09-22)*
- [x] `independent-windows-memory-owner` — checked Windows starter breakaway and native process-exit/survivor fixtures *(archived 2026-09-22)*

## Soft-blocked by

None.

## Downstream phase gates

`durable-service-operation-receipts` is a prerequisite for task 2.3's ordinary managed writable CLI switch and task 3.1's exact write-outcome claim. The unrelated facade, attachment, inspection and idle-owner source can progress while it is implemented, but this change cannot archive or expose managed writes until the receipt prerequisite is archived and integrated. The remaining P28 advisory/concurrent mutation proof follows this service; P29 depends on that proof and P09's summary provenance contract. P30 may admit concurrent conversation drivers only after P29 proves private-history isolation.
