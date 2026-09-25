# Dependencies

## Blocked by

- [x] `project-memory-service` — supplies the managed per-project facade, attachment lifecycle, exact recovery path, and final shared CLI/TUI integration on which P11's session reads and mutations run *(archived 2026-09-23)*
- [x] `durable-service-operation-receipts` — supplies request-bound accepted-write recovery for managed lifecycle mutations *(archived 2026-09-22)*
- [x] `session-scoped-memory-provenance` — supplies physical session attribution and the runtime rule that raw private history never crosses sessions *(archived 2026-09-22)*
- [x] `session-cursor-history-window` — supplies the bounded sequenced suffix-after-cursor read consumed by context compaction and preserves the session/view/revision boundary P11 must not weaken *(archived 2026-09-22)*
- [x] `atomic-compact-checkpoint-provenance` — establishes main schema v6, truthful operation-attributed context/private provenance and the atomic checkpoint API that P11's v7 migration must preserve unchanged *(archived 2026-09-22)*
- [x] `command-registry` — supplies the one typed slash-command catalog P11 extends *(archived 2026-09-22)*
- [x] `typed-message-foundation` — supplies the canonical typed content records P11 exports and forks without reducing them to text *(archived 2026-09-16)*

## Soft-blocked by

None.

## Coordinated siblings and downstream gate

P08 private reasoning summaries and P09 actor compaction retain their own private summary rows and context policy; P11 consumes only the public transcript projection, so neither is a delivery prerequisite. Their privacy fixtures must still remain green when fork and export land. P30 follows P11 and adds live-session registrations, atomic per-session driver claims and default concurrent new-session admission; P11 deliberately retains the current single-driver boundary.
