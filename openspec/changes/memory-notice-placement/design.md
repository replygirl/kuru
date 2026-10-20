# Design

## Context

`Transcript::push` appends local items with no position memory. `install_projection` (used by `install_newest` for Newest/Restore reads and by search hits) replaced the transcript with the incoming page and then appended every retained local at the end. Submitting a prompt calls `Navigation::follow()`, which requests a newest page, and each completed turn installs one again, so locals drift to just above the in-flight turn. The startup memory notice is one such local item, which is why it "follows" the user. `install_older`, `install_newer`, `cursor_for` and `next_match` additionally assumed public items occupy indices `0..sum(page.items)` with locals as a contiguous tail.

## Goals / Non-Goals

**Goals:** locals stay where they were shown across page installs; the memory disclosure stays once per project and keeps its draw-then-record and never-persisted invariants, but leaves the transcript.

**Non-Goals:** the per-project marker, memory package, provider path, speaker labeling.

## Decisions

- Anchor is `Option<ItemId>`, captured at push as the last item whose id is not `ItemId::Local` (including bound in-flight user/answer items). Anchoring on the last non-`local` item was rejected: notices pushed after a completed answer would anchor to the previous turn and jump above the new user message. Bindings do not alter anchors because bound ids are already non-`Local` when later items are pushed.
- Every install first removes all locals, performs the existing public-only page arithmetic unchanged, then re-places locals in original order: duplicates of incoming records (bound turns) are dropped; present anchor → after the anchor and after locals already placed there; `None` → before public records; absent anchor → before the next later item from the old arrangement that survives, else at the end. The "next later survivor" fallback keeps a local whose anchor scrolled out of the newest window above that window instead of re-attaching it to the tail. The anchor itself is retained, so a later install that brings the anchor back restores the exact place.
- `cursor_for` maps public ordinals to pages and `next_match` scans only public items; locals still yield no cursor and no match.
- The memory indicator is a dedicated `View::memory_notice`, not `View::notice`: the transient slot expires after five seconds and could expire before anyone reads it. It takes precedence over a transient notice so the first completed frame always contains it before `record()`. It clears on a non-release key or paste only. The status line's empty-scene early return now also requires no memory indicator.
- `/status` reads a `View::memory_directory` captured once from `MemoryStore::status()` at loop start, escaped with the same `{:?}` formatting as the notice. A failed status read is not fatal: the chat starts and `/status` prints `Memory: unavailable`.
- The local bound evicts the oldest local by a monotonic creation sequence, because positions no longer imply age once unanchored or orphaned locals are re-placed ahead of later ones.
- The memory indicator deliberately keeps precedence over transient notices; a transient notice raised before the first key may expire unseen. Letting it pre-empt the indicator would allow `record()` to follow a frame that did not show the disclosure.

## Risks / Trade-offs

- [A transient notice raised before first input is hidden behind the indicator and can expire unseen] → accepted: the indicator clears on the first key and is the higher-value one-time disclosure.
- [A local whose anchor and all later items are gone lands at the end] → same as previous behavior, only in that degenerate case.
- [Sibling change edits `Item`] → one added field; resolve textually on rebase.

## Operational surface

Local terminal presentation only: no bind address, container/runner, secret, connection-limit or binary-version change. The TUI reads one existing `MemoryStore::status()` at loop start; headless stderr remains the only non-TUI output and stdout/JSON is untouched. Native terminal behavior is exercised by the existing real-PTY smoke on each supported platform in CI.
