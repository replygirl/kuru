## 1. Provenance and private record model

- [x] 1.1 Depend on P04's committed `ToolInvocationContext` at the runtime admission boundary and thread its real session, turn, actor, and invocation values to the summary writer; verify a focused runtime fixture observes the exact tuple without a Harness operation ID.
- [x] 1.2 Define the versioned private summary record, item-coordinate validation, deterministic key, and idempotent store operation; verify memory unit and real-Dolt reopen fixtures accept one exact record and reject conflicting or malformed identity.

## 2. Settled provider integration

- [x] 2.1 Capture only reconciled successful typed reasoning-summary items after provider completion settles; verify deterministic provider completion, summary-free completion, and terminal failure fixtures produce the required record counts.
- [x] 2.2 Keep raw and settled reasoning summaries out of public progress and runtime-event detail. Any future display projection remains a separate, bounded consumer and never carries the private payload or provider coordinates.

## 3. Privacy boundaries and evidence

- [x] 3.1 Exclude producer-private summaries from the current transcript, peer/public prompt and context assembly, and automatic cross-session continuity; preserve producer-owned policy-selected replay or compaction as an explicit private decision rather than automatic injection. The invoking owner's full-memory export/restore retains them without projecting them to public conversation output. A later session-export or fork presentation capability owns its explicit no-sweep acceptance.
- [x] 3.2 Run the planned provider behavior evaluation and package/documentation checks; record observed evidence in the verification ledger and archive after the scoped checks resolve.
