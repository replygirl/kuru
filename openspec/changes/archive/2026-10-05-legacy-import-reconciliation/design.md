# Design

## Context

The current automatic migration creates and validates a per-project snapshot before activating a Dolt import. The current `run_backup` has a thirty-second no-progress bound that resets after each successful backup step. N4 adds concurrent session-driver admission and a native maintenance barrier, so explicit import must exclude both accepted work and draining cleanup. The older explicit-import branch contains useful inventory, remapping, typed-refusal, and no-retained-staging logic, but its `fill_candidate` imposes a thirty-second whole-copy deadline and predates the current admission/lifecycle design.

## Goals / Non-Goals

**Goals:**
- Selectively adapt the old explicit inventory/import behavior to current snapshot, store, and N4 ownership APIs.
- Keep original source and committed WAL intact, and retain a validated deterministic receipt.
- Make every definite pre-activation refusal typed and leave no new staging artifacts.
- Hold maintenance ownership through actual local connection close and child reap, including cancellation of the initiating caller.

**Non-Goals:**
- Change schema, migration registry, provider behavior, credentials, dependency pins, or mise configuration.
- Rework generic service admission or lease policy beyond a typed native contention result and the import cleanup lifetime.
- Replace automatic first-open scope behavior or introduce an unbounded/generic inventory framework.

## Decisions

- **Use existing bounded SQLite backup and snapshot validation.** Reusing the current `run_backup` retains its progress-resetting stall semantics. The alternative whole-copy deadline is rejected because total copy time grows with valid data size and says nothing about source stalling.
- **Separate pure inventory from destination activation.** Inventory reads and validates one bounded consistent source view and inspects checked status metadata; it does not start Dolt or open every project. The alternative of opening each target is rejected because it would mutate lifecycle state and scale work with unrelated projects.
- **Acquire native admission before staging, then recheck.** The maintenance barrier and service permit exclude admitted/draining drivers and starters. The destination is revalidated while those resources are held before snapshots or candidates are created. The alternative preflight-only check is racy with new session admission.
- **Retain a background owner for cancellation-sensitive import work.** A spawned owner carries the maintenance permit through local SQLite/Dolt close and reap; dropping the command future cannot release locks early. The alternative caller-owned future would unlock while child cleanup might still be running.
- **Represent native lock contention as a typed internal outcome.** The import maps `WouldBlock` to a definite no-effect refusal while propagating other I/O errors. Parsing platform error strings is rejected because it is neither stable nor semantically safe.
- **Preserve row identity conservatively.** Only a leading project-scope prefix is rewritten. Unknown moved-scope families and collisions refuse before activation; same-scope behavior retains compatibility. No opaque payload is heuristically rewritten.

## Risks / Trade-offs

- [Risk] A process may be cancelled after commit but before acknowledgement. → Keep the owned worker and durable receipt; return uncertainty unless checked reconciliation establishes the outcome.
- [Risk] SQLite may update SHM coordination state while observing WAL. → Promise byte preservation only for the database and WAL; describe SHM as rebuildable coordination data.
- [Risk] Inventory limits may reject unusually large legacy installations. → Return a typed bounded refusal with cause/remedy and leave the source and project stores untouched.

## Operational surface

This is a local CLI and library feature; it adds no service endpoint, listening socket, container, required secret, binary version, or supported architecture. Inventory and import use the existing local memory service and native per-project maintenance/session locks. Import remains bounded by the existing startup/close and snapshot-progress limits; it does not start provider traffic or contact a remote service.
