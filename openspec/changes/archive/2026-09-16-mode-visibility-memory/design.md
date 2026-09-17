## Context

P8 stores a validated profile with `Harness` and applies its first four axes at
runtime boundaries. Visibility and Memory remain pure core operations whose
reference results are still reproduced by fixed runtime paths. P7 already owns
the request-fit estimate and permanent usage ledger; P6 owns tool and outbound
A2A authorization.

## Goals / Non-Goals

**Goals:**

- Make Visibility and Memory decisions effective only at their existing runtime
  reads, writes, delivery and dream boundaries.
- Keep policy output untrusted until checked against the live topology,
  requested identity and mandatory P7 request material.
- Preserve the selected profile through existing durable publication and
  candidate reconciliation.

**Non-Goals:**

- A policy parser, shipping alternate mode, new UI or configuration surface.
- History deletion, automatic expiry, memory migration, new provider behavior
  or any Phase 2+ context-compaction architecture.

## Decisions

### Select once per actor request and reuse the validated source plan

Runtime will call `context_sources` at request assembly and use the same
validated plan for private/public reads and P7 source accounting. Reading fixed
context and filtering it later was rejected because the resulting provider
payload and accounting estimate could differ.

### Keep mandatory current material outside optional policy omission

`ExplicitInput` is required by core validation; current input, receipts and
native continuation remain runtime-owned mandatory material. Allowing a policy
to omit them was rejected because it could break tool coherence or silently
change a current request.

### Resolve every memory path through the selected profile

Internal profile-taking helpers will cover actor namespace, transcript,
topology/state and undo/dream keys. Built-in wrappers remain for standalone
inspection APIs. A global namespace switch was rejected because an uncertain
write could read from one profile and publish another.

### Validate consolidation from current active topology each time

The runtime will use the returned ordered participant subset but validate it at
the dream boundary and preserve typed Dream phase/accounting. Restricting plans
to every seed was rejected because it makes an alternate memory policy inert;
trusting plans without live validation was rejected because retired or repeated
parts could receive provider work.

## Risks / Trade-offs

- [A policy can request cross-private context] → validate before every selected
  read and fail before provider dispatch.
- [Memory routing can diverge across recovery] → carry the same profile through
  pending publication, candidate and reconcile paths.
- [Visibility omission can hide required material] → keep current input,
  receipts and continuation runtime-owned and include them in P7 accounting.
