## Context

Core messages have string content; completions split text and calls. SQL stores
role/content strings. Native Responses continuation retains raw actor-local
input/output, including encrypted reasoning. Memory production schema is v2,
with an unrelated test-only v3 migration fixture. See proposal and delta specs
for scope and requirements.

## Goals / Non-Goals

**Goals:** one block representation with deliberate legacy projections; preserve
old data and protocol continuation while replacing new string-encoded receipts.

**Non-Goals:** streaming, new reasoning persistence, media input, permission or
mode behavior, context allocation, new dependencies or backward-writing support.

## Decisions

1. Use `Message { role, blocks }` and completion blocks plus usage/stop metadata.
   Derive text/call accessors. Reject duplicated shadow text/call fields because
   they lose interleaving or diverge. Preserve unknown roles/stop strings;
   explicitly reject unknown block tags rather than add an opaque dispatch model.
2. Add production schema v3 `messages.content_format`, defaulting existing rows
   to `text-v1`. `typed-v1` encodes `{ "blocks": [...] }` in existing LONGTEXT;
   role stays its SQL column. Typed API writes use this single encoder even for
   text-only messages; raw text append/import retain `text-v1`. Reject string
   sniffing and rewriting old revisions because literal user text is ambiguous
   and old history must remain intact. Old-schema views retain text operations
   but reject typed writes until an appropriate upgraded view is used.
3. Select columns/codec from each branch's validated schema. Move the test-only
   future migration to v4 and replace implicit current=v2 setups with explicit
   old-schema fixtures. Reject numeric `found >= 3` test-marker assumptions.
4. Keep native Pending input/output transient. Typed current receipts map directly
   to pending IDs in native order; legacy tool-string conversion stays narrowly
   at its existing live continuation boundary. Never turn encrypted native state
   into a persisted reasoning-summary block. The runtime carries an explicit
   transient current-input boundary after bounding history: optional history may
   omit an assistant record, and peer input may appear between current receipts,
   so message adjacency cannot reliably identify the current receipt batch.
5. Export exact stored payloads with their discriminator and bump the outer export
   format. Derive Markdown/inspection text intentionally and identify structured
   blocks. Keep final TurnOutput/journal JSON compatible rather than changing that
   separate public contract as a side effect of the core Rust API change.
6. Preserve receipt-aware truncation, redaction markers, UTF-8 and complete tool
   identity. Bound encoded typed payloads; do not byte-truncate JSON into malformed
   blocks. This is preservation of current bounds, not token accounting.

## Operational surface

The existing native CLI and terminal are the affected interactive surfaces;
their text projection and final JSON answer remain compatible. No listener,
container, connection limit, credential, binary target or installation topology
changes. Tests use isolated fake credentials and local fixtures. The existing
embedded verified Dolt engine and native CI remain the storage/runtime substrate.

## Integration contract

The core package owns the block types. Memory owns the schema-v3 discriminator,
view-version codec and revision-pinned exports. Connectors own native request
translation and actor-local opaque continuation; the runtime owns final-answer
publication. HTTP fixtures verify exact call-ID reconciliation and output order
for API-key and subscription routes. The outer memory export advances its format
version; final TurnOutput serialization does not. No route, SDK, mount, provider
authentication or network ownership changes are part of this slice.

## Risks / Trade-offs

- [Cross-package API churn] → coordinate a fixed API among core/runtime, memory,
  connectors/TUI owners and compile all targets after integration.
- [Old schema and new readers] → actual Dolt upgrade/candidate/revision fixtures
  and explicit old-binary rejection, not a data rewrite or downgrade.
- [Changed native inference inputs] → local HTTP fixtures compare continuation
  order, raw opaque state and plain-text projections on both routes.
- [Misleading export or visible output] → JSON/Markdown and PTY/CLI evidence,
  unchanged final answer fields and exact-retry regression fixtures.
