## Why

Kuru stores every message as a string and reconstructs tool protocol objects from
JSON embedded in that string. This prevents the Phase 1 streaming, context and
permission foundations from sharing an ordered, typed conversation contract.

## What Changes

- BREAKING: make messages and completions use canonical ordered content blocks
  for text, tool use/results, reasoning and images with explicit usage,
  stop-reason and cache metadata rather than independent text/call representations.
- Add an explicit durable content-format discriminator using the existing ordered
  migration mechanism; retain supported historical rows and candidate views.
- Adapt native provider requests, runtime receipts, memory export/inspection and
  existing CLI/TUI text projections without changing inference policy or final
  answer authority.
- Keep opaque native reasoning continuation private and transient. This foundation
  does not add reasoning-summary persistence, streaming, media input, permission
  behavior, model metadata, context accounting or mode-policy changes.

## Capabilities

### New Capabilities

- `content-block-messages`: ordered content/metadata types, explicit durable codec
  discrimination, supported legacy decoding and safe text projections.

### Modified Capabilities

- `versioned-memory`: typed message storage and export with historical schema
  compatibility, candidate isolation and atomic message/state checkpoints.
- `provider-tools`: native typed tool call/result translation and preservation of
  actor-scoped transient continuation state.

## Impact

Core Rust message/completion APIs change (BREAKING). Memory introduces an ordered
schema migration; older binaries must reject the newer schema, while new binaries
continue reading supported old revisions. Provider, runtime and TUI call sites and
fixtures migrate together. Existing final CLI answer fields, mode identities,
workspace trust and no-expiry memory semantics stay compatible. No new dependency
or language toolchain is planned.

## Surfaces

- [x] interactive — CLI/TUI text and inspection/export projections
- [ ] deploy — no runtime or CI topology change
- [x] integration — native provider and durable/export data contracts
- [x] agent-behavior — typed message/tool output shape, with existing behavior retained
