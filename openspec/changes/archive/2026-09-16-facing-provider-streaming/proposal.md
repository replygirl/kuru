## Why

Providers currently collect complete replies before the runtime can expose any
answer text. Phase 1 requires a usable provisional facing preview while keeping
private peer work, native continuation state and durable completed answers separate.

## What Changes

- Make a normalized provider stream the canonical inference path, with completion
  collected from its single successful terminal outcome. BREAKING: provider
  implementations implement `stream` instead of `complete`.
- Surface bounded native SSE text, visible summary, tool-fragment, usage and
  terminal observations on both existing OpenAI routes; preserve native continuation
  privacy, cancellation, limits and final typed tool authorization.
- Carry bounded latest-value previews only from the selected speaking invocation
  to the TUI, including a selected relationship speaker. Render partial text and
  visible summaries or truthful activity separately from saved conversation.
- Settle previews exclusively from the final turn output; cancellation, retry and
  resume never persist or replay provisional text. Keep scripted output final-only.

## Capabilities

### New Capabilities

### Modified Capabilities

- `provider-tools`: normalized streaming, terminal collection and native fragment
  reconciliation, with explicit usage fidelity.
- `chat-harness`: bounded provisional facing previews, responsive rendering and
  stale-operation fencing.
- `content-block-messages`: allow transient visible summaries and streaming while
  retaining final typed content authority and no new durable summary capture.

## Impact

Connector provider/SSE interfaces, runtime actor work and selected-speaking path,
TUI scheduling/rendering, provider fixtures and user documentation change. No
database migration, new provider, permission rule, cost ledger, mode dynamics,
durable token log or persisted-reasoning product is introduced. Typed semantic
events from P4 remain separate from transient preview transport.

## Surfaces

- [x] interactive — partial-line and visible-summary TUI preview
- [ ] deploy — no deployment topology change
- [x] integration — existing native Responses and ChatGPT SSE transports
- [x] agent-behavior — provider output delivery and final tool authorization
