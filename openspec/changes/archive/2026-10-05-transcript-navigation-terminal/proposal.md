# Proposal

## Why

Phase 2 U5 requires usable long public scrollback, mouse navigation, transcript search and safe terminal integration. The current TUI projects one 500-record page into tuples, loses continuation and item identities, and tracks reading by a global u16 distance from the tail, so paging and variable-height updates cannot preserve a reader's place.

## What Changes

- Retain selected-session public pages, exact provenance/cursors and stable message identities in a bounded view window; navigate outside the initial viewport without loading the whole session.
- Replace history-wide numeric scrolling with item/within-item reading anchors and a distinct follow-tail state; render viewport rows with bounded per-item layout caches.
- Add incremental cancellable literal public-message search with visible scanned, found, completed, cancelled and stale/unavailable states and exact session/view/query epochs.
- Enable mouse capture by default through the existing terminal guard while preserving approval, picker, recall, composer and tool-card priority.
- Add sanitized restorable session titles only for supported mechanisms and fixed content-free out-of-focus signals for actual turn completion; restore owned state on all exits.
- Move existing public-page traversal onto a proven selected-view immutable commit pool outside the writer mutex, reading structural metadata across the chain and full bodies only for the bounded returned page; preserve current cursor invalidation and all returned-message checks.
- Document public search scope, omission counts, navigation controls and terminal capability limits, and verify real PTYs at 80/120 columns.

## Capabilities

### New Capabilities

- `interactive-transcript-navigation`: bounded selected-session public paging, stable reading/search and owned mouse/title/completion integration.

### Modified Capabilities

None. Existing public transcript storage, transient cards, composer, Event, journal and TurnOutput contracts remain unchanged.

## Impact

TUI public projection adapter, ui.rs and narrow navigation/render/tool-card anchoring helpers, terminal guard and fixtures, existing memory-owned public reader and its fixtures, feature documentation and this change's artifacts. No memory schema/index, private history search, durable SQL cards/outbox, provider effects, dependencies, U4 theme or general input/data framework. Read-only memory seams remain behind the existing app adapter. The narrow reader implementation changes no public API, cursor authority, wire contract or schema; remaining repeated metadata-query cost is explicit.

## Surfaces

- [x] interactive — transcript navigation/search and terminal UX
- [ ] deploy — unchanged install and runtime topology
- [x] integration — existing public-page cursor and terminal-control contracts
- [ ] agent-behavior — existing actor prompts, routes and effects retained
