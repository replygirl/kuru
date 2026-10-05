# Design

## Context

`View` currently owns the canonical UTF-8 draft and a byte cursor. Key editing and `editor_layout` walk Unicode scalar values, paste appends scalars incrementally, prompt recall is absent, and the transcript render clamps its visible offset while `View.scroll` itself can grow without bound. The active U1 `/config` dispatch and existing picker, rename, instruction, and approval branches share this event loop and remain part of its behavior.

## Goals / Non-Goals

**Goals:** Keep one canonical draft, keep cursor targets on complete global grapheme boundaries and chip offsets on exact UTF-8 boundaries, bound all recall and paste projection state, preserve selected-session ownership and modal priority, and derive cursor/scroll rendering from the visible content.

**Non-Goals:** Durable prompt history, transcript-backed recall, memory/runtime schema or API changes, P15 tool cards, a generic editor framework, CLI grammar changes, themes, and transcript-search/navigation features.

## Decisions

1. Keep the existing draft string and byte cursor in `View`, with `unicode-segmentation` helpers in a small `ui/composer.rs` module. Use grapheme indices for horizontal, line-local, and vertical motion, and existing terminal-width measurement on complete clusters for rendering. Cursor targets are global grapheme boundaries; raw draft edits still use UTF-8 byte ranges. Rejected scalar-index movement because combining and joined sequences would split; rejected a separate editor state owner because the current event loop must preserve modal and command priority.
2. Store paste chips as bounded byte ranges over the canonical string. A paste may start or end inside the grapheme segmentation of the full draft when its combining/ZWJ text joins neighboring content, so chip selection must resolve an adjacent cursor grapheme by overlap while retaining the original chip byte range. Render a separate bounded projection; expansion affects display only, edits reveal an intersected collapsed span first, and explicit removal deletes exactly the recorded pasted bytes without widening to the joined grapheme. Reject an over-limit paste before modifying the string. Rejected replacement tokens or detached paste storage because either can make visible and submitted prompt content diverge.
3. Store submitted prompts in a per-`View`, bounded in-memory history keyed by session ID. Record at the existing prompt dispatch boundary under the currently selected session, before async settlement; browsing never reads transcripts or memory. Recall/search hold a saved unsent draft until explicit selection and restore it on cancellation. Rejected persistence and settlement-time attribution because history is an ephemeral composer aid and the submitted prompt already has an unambiguous dispatch owner.
4. Derive the composer viewport and cursor from one projected layout on every frame, and clamp transcript scroll against the currently rendered line count and viewport. Recompute after edits and terminal resize. Keep approval, instruction, and picker dispatch ahead of recall and ordinary editing. Rejected independent scroll/cursor caches because size or content changes can stale their bounds.

## Risks / Trade-offs

- Grapheme segmentation and terminal cell width can differ for unusual emoji or terminal fonts → place the cursor using whole-cluster boundaries, clamp to viewport cells, and verify combining/ZWJ examples through deterministic renders and PTYs at practical widths.
- Paste boundaries can fall within a grapheme after neighboring bytes join → keep chip offsets as exact paste byte spans, resolve chip actions by overlapping adjacent grapheme ranges, and test expansion/removal where pasted combining or ZWJ bytes join surrounding text.
- A collapsed chip can hide meaningful pasted content → show size and line cues, provide a discoverable expand/remove hint, and keep its complete literal content available before edits.
- Bounded in-memory history loses old entries through eviction or process exit → disclose eviction and document its application-lifetime scope without implying transcript persistence.
- Several input modes share one key dispatcher → preserve existing modal-first branches and verify denied approval choices, draft restoration, and ordinary input after dismissal.

## Operational surface

This change runs only in the existing local `kuru` TUI process. It adds no bind address, container, hosted runner behavior, required secret, connection limit, executable target, new dependency version, or supported architecture. The TUI declares direct workspace use of the existing exact-pinned `unicode-segmentation` crate; the existing terminal event loop and native console restoration remain authoritative, with acceptance through real terminal PTYs at practical widths.
