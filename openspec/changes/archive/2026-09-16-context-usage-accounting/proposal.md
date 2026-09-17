## Why

Kuru currently reports turn-level token values without a durable record for every provider invocation. Deliberation, consultation, continuation and dreams can consume tokens even when a turn fails or a dream candidate is abandoned, so session totals and cost presentation cannot be inferred honestly from completed answers.

Context fitting also sees less than the effective native continuation. A request can retain opaque actor-local pending items and tool receipts that are absent from the runtime's visible-message estimate; Phase 1 needs a pre-dispatch fit check on the actual request while preserving private continuation and durable history.

## What Changes

- Add a narrow, per-project durable usage ledger on a permanent operational Dolt branch, separate from live and dream-candidate revision history. Admit each provider invocation before dispatch, durably observe ordered usage, settle failures and cancellations without inventing zeros, and fold session totals from invocation records.
- Preflight each effective connector request after native continuation selection and before network dispatch. Meter all mandatory material and source-labelled optional history against the selected model window and output reserve; omit only whole older optional rows and disclose actual omissions. Unknown windows use a labelled conservative assumption, never a setup gate.
- Show `/cost`, incomplete/estimated session totals and an estimated per-request context indicator. Preserve frozen price basis and distinguish subscription API-equivalent estimates from charges or quota.
- Reuse the P2 `assumed_context_window_tokens` setting and add only a bounded output-reserve override to configuration and its published schema. Keep existing typed messages, event journal, permission policy and mode behavior authoritative.

## Capabilities

### New Capabilities

- `context-usage-accounting`: Durable invocation accounting, effective-request fit, truthful cost/context presentation and their boundaries.

### Modified Capabilities

- `memory-store-lifecycle`: The permanent operational usage branch follows owned open, reconciliation, close, purge and export lifecycle rules without advancing live or candidate history.
- `provider-tools`: Connector preflight measures the actual selected native continuation and refuses oversized mandatory requests before HTTP while delivering ordered usage observations.
- `chat-harness`: Session `/cost` and status expose estimates, incomplete history and actual optional-context omissions.
- `configuration-schema`: New context settings retain strict parser/schema parity and documented bounds.

## Impact

- `packages/kuru-memory` owns the operational branch, narrow `UsageLedger` API, schema/open/recovery/export lifecycle and real-Dolt tests; no general writable branch handle escapes.
- `packages/kuru-core` owns bounded context settings and typed accounting/fit contracts; `apps/kuru-docs/public/configuration.v1.schema.json` tracks the parser.
- `packages/kuru-connectors` owns effective request construction/preflight and ordered, fallible usage delivery without exposing native pending content.
- `packages/kuru-runtime` owns invocation identities, mandatory/optional source inventory, whole-row omission, output reserve, usage settlement and session projections. `apps/kuru-tui` owns `/cost` and estimated status presentation; docs explain the limits.
- Existing project stores gain one permanent Dolt branch. Its owned ledger contract is versioned independently of unrelated main-branch message migrations. No existing history is rewritten; pre-ledger sessions remain visibly incomplete.

## Surfaces

- [x] interactive — `/cost`, status estimates and context-omission notices
- [ ] deploy — no deployment topology or workflow change
- [x] integration — provider usage observations and effective native request preflight
- [x] agent-behavior — optional prompt-history selection and pre-dispatch refusal
