# Dependencies

## Blocked by

- [x] `durable-service-operation-receipts` — exact-view authenticated mutation receipts and lost-reply reconciliation *(archived 2026-09-22)*

## Soft-blocked by

None.

## Delivery ordering

The exact reviewed `project-memory-service` implementation commit is branch ancestry, so source implementation can proceed without a placeholder backend. That change must still archive and merge before this dependent change is delivered. `actor-context-compaction` consumes this change and is not an upstream dependency.

# Follow-on P29 cohort

This first storage slice does not complete P29. Before Phase 2 closes, a later
P29 slice must integrate per-turn topology reload, policy-selected continuity
across attributed private histories and admitted shared notes, and serialized
dream publication with explicit live-memory conflict preservation. P30 remains
the separate gate for concurrent conversation-driver admission.
